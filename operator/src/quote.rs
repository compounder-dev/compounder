//! The issuer's signed quote: the price at which the token's issuer stands
//! ready to trade right now. The settlement contract holds every batch price
//! to one, and checks the signature itself.
//!
//! A market route that trades with the issuer carries the quote inside its
//! call, so it is read from there. Nothing here is trusted: a quote that is
//! not the issuer's is refused by the contract.

use alloy::primitives::{keccak256, Address, Bytes, Signature, B256, U256};

use crate::ISettlement;

const WORD: usize = 32;
const FIELDS: usize = 9;
const SIGNATURE: usize = 65;
const QUOTE_TYPE: &str = "Quote(uint256 chainId,uint256 attestationId,bytes32 userId,address asset,uint256 price,uint256 quantity,uint256 expiration,uint8 side,bytes32 additionalData)";

/// A quote found in a call, with one signature that fits it and who signed.
/// Whether that signer speaks for the issuer is for the caller to ask.
pub struct Candidate {
    pub quote: ISettlement::Quote,
    pub signature: Bytes,
    pub signer: Address,
}

fn word(call: &[u8], at: usize) -> U256 {
    U256::from_be_slice(&call[at..at + WORD])
}

/// Every quote for `token` in `call`, in the wanted direction, paired with
/// each signature in the call that could be for it. The quote sits in the
/// call as its nine fields in a row, and a signature as 65 bytes after a
/// word that gives that length.
pub fn candidates(call: &[u8], chain: u64, token: Address, issuer_sells: bool, domain: B256) -> Vec<Candidate> {
    let side = U256::from(if issuer_sells { 0u8 } else { 1u8 });
    let length = U256::from(SIGNATURE);
    let signatures: Vec<&[u8]> = (WORD..call.len().saturating_sub(SIGNATURE - 1))
        .filter(|&at| word(call, at - WORD) == length && matches!(call[at + SIGNATURE - 1], 27 | 28))
        .map(|at| &call[at..at + SIGNATURE])
        .collect();

    let mut found = Vec::new();
    for at in 0..call.len().saturating_sub(FIELDS * WORD - 1) {
        let field = |index: usize| word(call, at + index * WORD);
        if field(0) != U256::from(chain) || field(3) != U256::from_be_slice(token.as_slice()) || field(7) != side {
            continue;
        }
        let mut signed = Vec::with_capacity((FIELDS + 1) * WORD);
        signed.extend_from_slice(keccak256(QUOTE_TYPE).as_slice());
        signed.extend_from_slice(&call[at..at + FIELDS * WORD]);
        let mut message = vec![0x19, 0x01];
        message.extend_from_slice(domain.as_slice());
        message.extend_from_slice(keccak256(&signed).as_slice());
        let digest = keccak256(&message);

        for bytes in &signatures {
            let Ok(signer) = Signature::from_raw(bytes).and_then(|signature| signature.recover_address_from_prehash(&digest)) else {
                continue;
            };
            found.push(Candidate {
                quote: ISettlement::Quote {
                    chainId: field(0),
                    attestationId: field(1),
                    userId: B256::from(field(2)),
                    asset: token,
                    price: field(4),
                    quantity: field(5),
                    expiration: field(6),
                    side: if issuer_sells { 0 } else { 1 },
                    additionalData: B256::from(field(8)),
                },
                signature: Bytes::copy_from_slice(bytes),
                signer,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};

    const CHAIN: u64 = 56;

    fn token() -> Address {
        Address::repeat_byte(0xa9)
    }

    fn fields(token: Address, side: u8, price: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let values = [
            U256::from(CHAIN),
            U256::from(0x1234_5678u64),
            U256::from(7u8) << 200,
            U256::from_be_slice(token.as_slice()),
            U256::from(price),
            U256::from(60u8),
            U256::from(1_800_000_000u64),
            U256::from(side),
            U256::ZERO,
        ];
        for value in values {
            out.extend_from_slice(&value.to_be_bytes::<32>());
        }
        out
    }

    fn digest(domain: B256, fields: &[u8]) -> B256 {
        let mut signed = keccak256(QUOTE_TYPE).to_vec();
        signed.extend_from_slice(fields);
        let mut message = vec![0x19, 0x01];
        message.extend_from_slice(domain.as_slice());
        message.extend_from_slice(keccak256(&signed).as_slice());
        keccak256(&message)
    }

    /// A call shaped like a real route: other data, the quote, more data, and
    /// two signatures of which only one is over the quote.
    fn call_with(fields: &[u8], signatures: &[Vec<u8>]) -> Vec<u8> {
        let mut call = vec![0xde, 0xad, 0xbe, 0xef];
        call.extend_from_slice(&[0x11; 77]);
        call.extend_from_slice(fields);
        call.extend_from_slice(&[0x22; 40]);
        for signature in signatures {
            call.extend_from_slice(&U256::from(SIGNATURE).to_be_bytes::<32>());
            call.extend_from_slice(signature);
            call.extend_from_slice(&[0u8; 31]);
        }
        call
    }

    #[test]
    fn the_quote_and_its_signer_are_found_among_other_data() {
        let domain = B256::repeat_byte(0x5a);
        let issuer = PrivateKeySigner::random();
        let venue = PrivateKeySigner::random();
        let quote = fields(token(), 1, 231);
        let by_issuer = issuer.sign_hash_sync(&digest(domain, &quote)).unwrap().as_bytes().to_vec();
        let by_venue = venue.sign_hash_sync(&B256::repeat_byte(0x77)).unwrap().as_bytes().to_vec();
        let call = call_with(&quote, &[by_venue, by_issuer.clone()]);

        let found = candidates(&call, CHAIN, token(), false, domain);
        let ours: Vec<_> = found.iter().filter(|candidate| candidate.signer == issuer.address()).collect();
        assert_eq!(ours.len(), 1, "the issuer's signature was not matched to its quote");
        assert_eq!(ours[0].signature.as_ref(), by_issuer.as_slice());
        assert_eq!(ours[0].quote.price, U256::from(231u64));
        assert_eq!(ours[0].quote.quantity, U256::from(60u8));
        assert_eq!(ours[0].quote.side, 1);
        assert!(found.iter().all(|candidate| candidate.signer != venue.address()), "a signature over something else was taken for the quote's");
    }

    #[test]
    fn a_quote_for_another_token_direction_or_chain_is_not_found() {
        let domain = B256::repeat_byte(0x5a);
        let issuer = PrivateKeySigner::random();
        let quote = fields(token(), 1, 231);
        let call = call_with(&quote, &[issuer.sign_hash_sync(&digest(domain, &quote)).unwrap().as_bytes().to_vec()]);
        assert!(candidates(&call, CHAIN, Address::repeat_byte(0xb2), false, domain).is_empty());
        assert!(candidates(&call, CHAIN, token(), true, domain).is_empty());
        assert!(candidates(&call, 1, token(), false, domain).is_empty());
        assert!(candidates(&[0u8; 10], CHAIN, token(), false, domain).is_empty());
    }
}
