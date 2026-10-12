import { useEffect, useState, type ReactNode } from 'react';
import { Button, Field, Icon, Segmented, Sheet } from '../parts/index.ts';
import {
  COMPARATORS,
  FREQUENCIES,
  INDICATORS,
  evenShares,
  listed,
  symbolOf,
  weightedBy,
  type AssetRef,
  type Block,
  type Comparator,
  type Comparison,
  type Condition,
  type FilterBlock,
  type Frequency,
  type IfBlock,
  type Indicator,
  type Measure,
  type Reading,
  type Rebalance,
  type WeightBlock,
  type Weighting,
} from './draft.ts';
import { searched, useUniverse, type Listed } from './universe.ts';
import { BLOCK_KINDS, COMPARATOR_WORDS, FREQUENCY_WORDS, MEASURES, comparisonWords, partsOf } from './words.ts';

// ---- small controls ----

/** A number typed in place. What is typed stays as typed, and each number it can be read as is handed on. */
function NumberInput({ label, value, onChange, min, max, whole = false, suffix }: { label: string; value: number; onChange: (value: number) => void; min?: number; max?: number; whole?: boolean; suffix?: string }) {
  const [text, setText] = useState(String(value));
  // A change from outside, such as an undo, replaces what was typed.
  useEffect(() => {
    setText((typed) => (Number(typed) === value ? typed : String(value)));
  }, [value]);
  const type = (typed: string) => {
    setText(typed);
    const read = Number(typed);
    if (typed.trim() === '' || !Number.isFinite(read)) return;
    const kept = Math.min(max ?? Infinity, Math.max(min ?? -Infinity, whole ? Math.round(read) : read));
    onChange(kept);
  };
  return (
    <label className="form-field">
      <span className="form-label">{label}</span>
      <span className="form-box">
        <input className="form-input" inputMode={whole ? 'numeric' : 'decimal'} value={text} onChange={(event) => type(event.target.value)} />
        {suffix !== undefined && <span className="form-suffix">{suffix}</span>}
      </span>
    </label>
  );
}

/** A share typed as a percent and kept as a fraction: 60 is 0.6. */
function PercentInput({ label, value, onChange, max = 100 }: { label: string; value: number; onChange: (value: number) => void; max?: number }) {
  return <NumberInput label={label} value={Number((value * 100).toFixed(4))} min={0} max={max} suffix="%" onChange={(percent) => onChange(Number((percent / 100).toFixed(6)))} />;
}

function Choice<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: { value: T; label: string }[]; onChange: (value: T) => void }) {
  return (
    <label className="form-field">
      <span className="form-label">{label}</span>
      <span className="form-box">
        <select className="form-input" value={value} onChange={(event) => onChange(event.target.value as T)}>
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </span>
    </label>
  );
}

const MEASURE_OPTIONS = INDICATORS.map((indicator) => ({ value: indicator, label: MEASURES[indicator].name }));
const DAYS = { min: 2, max: 1000, start: 20 };

/** A measure and the days it looks back over. One that has no days shows none. */
function MeasureFields({ measure, onChange, pick }: { measure: Measure; onChange: (measure: Measure) => void; pick?: Indicator[] }) {
  const options = pick ? MEASURE_OPTIONS.filter((option) => pick.includes(option.value)) : MEASURE_OPTIONS;
  const choose = (indicator: Indicator) => onChange(MEASURES[indicator].windowed ? { indicator, window: measure.window ?? DAYS.start } : { indicator });
  return (
    <>
      <Choice label="Measure" value={measure.indicator} options={options} onChange={choose} />
      {MEASURES[measure.indicator].windowed && <NumberInput label="Days" value={measure.window ?? DAYS.start} min={DAYS.min} max={DAYS.max} whole onChange={(window) => onChange({ ...measure, window })} />}
      <p className="form-hint">{MEASURES[measure.indicator].line}</p>
    </>
  );
}

/** The actions every block but the root has: move it within its list, or take it away. */
export type BlockActions = { up?: () => void; down?: () => void; remove?: () => void };

function Actions({ actions }: { actions: BlockActions }) {
  if (!actions.up && !actions.down && !actions.remove) return null;
  return (
    <div className="editor-actions">
      <Button icon="arrow-up" onClick={actions.up} disabled={!actions.up}>
        Move up
      </Button>
      <Button icon="arrow-down" onClick={actions.down} disabled={!actions.down}>
        Move down
      </Button>
      <Button variant="danger" icon="trash" onClick={actions.remove} disabled={!actions.remove}>
        Remove
      </Button>
    </div>
  );
}

// ---- the asset picker ----

type PickerProps = {
  open: boolean;
  title: string;
  /** With `several`, the sheet stays open for more picks and each tap adds one. */
  several?: boolean;
  /** Tickers already chosen, shown as chosen. */
  chosen?: string[];
  onPick: (symbol: string) => void;
  onClose: () => void;
};

/** Every share and fund there are prices for, to search and to pick from. */
export function AssetPicker({ open, title, several = false, chosen = [], onPick, onClose }: PickerProps) {
  const universe = useUniverse();
  const [typed, setTyped] = useState('');
  useEffect(() => {
    if (open) setTyped('');
  }, [open]);
  const found: Listed[] = universe.state === 'ready' ? searched(universe.assets, typed) : [];
  return (
    <Sheet open={open} onClose={onClose} title={title} variant="full" actions={several ? <Button variant="primary" fullWidth onClick={onClose}>Done</Button> : undefined}>
      <div className="picker">
        <Field label="Search by ticker or name" type="search" icon="search" placeholder="Search by ticker or name" value={typed} onChange={setTyped} />
        {universe.state === 'loading' && <p className="form-hint">Loading what there is to hold…</p>}
        {universe.state === 'failed' && <p className="form-hint">The list could not be loaded. Close this and try again.</p>}
        {universe.state === 'ready' && found.length === 0 && <p className="form-hint">Nothing matches. Try a ticker, such as SPY.</p>}
        <ul className="picker-list" role="list">
          {found.map((asset) => {
            const has = chosen.includes(asset.symbol);
            return (
              <li key={asset.symbol}>
                <button type="button" className="picker-row" aria-pressed={several ? has : undefined} onClick={() => onPick(asset.symbol)}>
                  <span className="ticker">{asset.symbol}</span>
                  <span className="picker-name">{asset.name ?? ''}</span>
                  {has && <Icon name="check" />}
                </button>
              </li>
            );
          })}
        </ul>
      </div>
    </Sheet>
  );
}

/** A button that names an asset and opens the picker to change it. */
function AssetButton({ label, asset, onChange }: { label: string; asset: AssetRef; onChange: (asset: AssetRef) => void }) {
  const [picking, setPicking] = useState(false);
  return (
    <div className="form-field">
      <span className="form-label">{label}</span>
      <button type="button" className="form-box form-pick" onClick={() => setPicking(true)}>
        <span className="ticker">{symbolOf(asset)}</span>
        <span className="form-pick-change">Change</span>
      </button>
      <AssetPicker
        open={picking}
        title={label}
        onClose={() => setPicking(false)}
        onPick={(symbol) => {
          onChange(listed(symbol).asset);
          setPicking(false);
        }}
      />
    </div>
  );
}

// ---- one editor for each kind of block ----

const WEIGHTINGS: { value: Weighting['type']; label: string }[] = [
  { value: 'equal', label: 'Equally' },
  { value: 'specified', label: 'Set shares' },
  { value: 'inverse_volatility', label: 'Calmer gets more' },
];

function SplitEditor({ block, onChange }: { block: WeightBlock; onChange: (block: WeightBlock) => void }) {
  const { weighting } = block;
  const total = weighting.type === 'specified' ? weighting.shares.reduce((sum, share) => sum + share, 0) : 1;
  const whole = Math.abs(total - 1) < 1e-6;
  return (
    <div className="editor">
      <Segmented label="How to split" name="split-how" items={WEIGHTINGS} value={weighting.type} onChange={(type) => onChange(weightedBy(block, type))} />
      {weighting.type === 'inverse_volatility' && (
        <>
          <NumberInput label="Days" value={weighting.window} min={DAYS.min} max={DAYS.max} whole onChange={(window) => onChange({ ...block, weighting: { type: 'inverse_volatility', window } })} />
          <p className="form-hint">Whichever has swung less over these days gets a larger share.</p>
        </>
      )}
      {weighting.type === 'specified' && (
        <>
          {block.children.length === 0 && <p className="form-hint">Add what it holds first, then set a share for each.</p>}
          {block.children.map((child, place) => (
            <PercentInput
              key={place}
              label={child.block === 'asset' ? symbolOf(child.asset) : `Rule ${place + 1}`}
              value={weighting.shares[place] ?? 0}
              onChange={(share) => onChange({ ...block, weighting: { type: 'specified', shares: block.children.map((_, index) => (index === place ? share : (weighting.shares[index] ?? 0))) } })}
            />
          ))}
          {block.children.length > 0 && (
            <p className={whole ? 'form-hint' : 'form-hint form-problem'} role="status">
              {whole ? 'The shares add up to 100%.' : `The shares add up to ${Number((total * 100).toFixed(2))}%. They need to add up to 100%.`}
            </p>
          )}
          {block.children.length > 0 && <Button onClick={() => onChange({ ...block, weighting: { type: 'specified', shares: evenShares(block.children.length) } })}>Make them equal</Button>}
        </>
      )}
    </div>
  );
}

function ComparisonEditor({ id, comparison, onChange }: { id: string; comparison: Comparison; onChange: (comparison: Comparison) => void }) {
  const { left, right } = comparison;
  const against = 'fixed' in right ? 'number' : 'measure';
  const percent = MEASURES[left.indicator].scale === 'percent';
  const setAgainst = (kind: 'number' | 'measure') => {
    if (kind === against) return;
    onChange({ ...comparison, right: kind === 'number' ? { fixed: percent ? 0.5 : 100 } : { ...left } });
  };
  const setLeft = (next: Reading) => {
    // A fixed number is in the scale of the measure it stands against. When the scale changes, it starts again.
    const rescaled = 'fixed' in right && MEASURES[next.indicator].scale !== MEASURES[left.indicator].scale;
    onChange({ ...comparison, left: next, right: rescaled ? { fixed: MEASURES[next.indicator].scale === 'percent' ? 0.5 : 100 } : right });
  };
  return (
    <fieldset className="editor-group">
      <legend className="editor-legend">{comparisonWords(comparison)}</legend>
      <AssetButton label="Of" asset={left.asset} onChange={(asset) => setLeft({ ...left, asset })} />
      <MeasureFields measure={left} onChange={(measure) => setLeft({ ...measure, asset: left.asset })} />
      <Choice label="Is" value={comparison.compare} options={COMPARATORS.map((value) => ({ value, label: COMPARATOR_WORDS[value] }))} onChange={(compare: Comparator) => onChange({ ...comparison, compare })} />
      <Segmented
        label="Compared with"
        name={`against-${id}`}
        items={[
          { value: 'number', label: 'A number' },
          { value: 'measure', label: 'Another measure' },
        ]}
        value={against}
        onChange={setAgainst}
      />
      {'fixed' in right ? (
        percent ? (
          <PercentInput label="Number" value={right.fixed} max={100000} onChange={(fixed) => onChange({ ...comparison, right: { fixed } })} />
        ) : (
          <NumberInput label="Number" value={right.fixed} min={0} suffix="$" onChange={(fixed) => onChange({ ...comparison, right: { fixed } })} />
        )
      ) : (
        <>
          <AssetButton label="Of" asset={right.asset} onChange={(asset) => onChange({ ...comparison, right: { ...right, asset } })} />
          <MeasureFields measure={right} onChange={(measure) => onChange({ ...comparison, right: { ...measure, asset: right.asset } })} />
        </>
      )}
    </fieldset>
  );
}

/** A condition to start from: a price above its own long average, on the asset given. */
export function startingComparison(symbol: string): Comparison {
  const asset = listed(symbol).asset;
  return { compare: 'above', left: { indicator: 'price', asset }, right: { indicator: 'average_price', window: 200, asset } };
}

function IfEditor({ block, onChange }: { block: IfBlock; onChange: (block: IfBlock) => void }) {
  const parts = partsOf(block.condition);
  const nested = parts.some((part) => 'any' in part || 'all' in part);
  const mode = 'any' in block.condition ? 'any' : 'all';
  // One comparison stands alone. Several are held in a group that says how many must be true.
  const grouped = (next: Condition[], as: 'any' | 'all'): Condition => (next.length === 1 ? next[0] : as === 'any' ? { any: next } : { all: next });
  const set = (next: Condition[], as: 'any' | 'all' = mode) => onChange({ ...block, condition: grouped(next, as) });
  if (nested) return <p className="form-hint">This condition has groups inside groups. It can be moved or removed here, and edited once groups can be.</p>;
  const first = parts[0] as Comparison;
  return (
    <div className="editor">
      {parts.length > 1 && (
        <Segmented
          label="How many must be true"
          name="if-mode"
          items={[
            { value: 'all', label: 'All of these' },
            { value: 'any', label: 'Any of these' },
          ]}
          value={mode}
          onChange={(as) => set(parts, as)}
        />
      )}
      {(parts as Comparison[]).map((part, place) => (
        <div key={place} className="editor-part">
          <ComparisonEditor id={String(place)} comparison={part} onChange={(next) => set(parts.map((each, index) => (index === place ? next : each)))} />
          {parts.length > 1 && (
            <Button variant="ghost" icon="trash" onClick={() => set(parts.filter((_, index) => index !== place))}>
              Remove this condition
            </Button>
          )}
        </div>
      ))}
      <Button icon="plus" onClick={() => set([...parts, startingComparison(symbolOf(first.left.asset))])}>
        Add a condition
      </Button>
    </div>
  );
}

function PickEditor({ block, onChange }: { block: FilterBlock; onChange: (block: FilterBlock) => void }) {
  return (
    <div className="editor">
      <Segmented
        label="Which to keep"
        name="pick-take"
        items={[
          { value: 'top', label: 'The highest' },
          { value: 'bottom', label: 'The lowest' },
        ]}
        value={block.take}
        onChange={(take) => onChange({ ...block, take })}
      />
      <NumberInput label="How many" value={block.count} min={1} max={Math.max(1, block.children.length)} whole onChange={(count) => onChange({ ...block, count })} />
      {/* The latest price cannot rank assets against each other: a dearer share is not a better one. */}
      <MeasureFields measure={block.sort_by} pick={INDICATORS.filter((indicator) => MEASURES[indicator].scale === 'percent')} onChange={(sort_by) => onChange({ ...block, sort_by })} />
    </div>
  );
}

const EDITOR_TITLES: Record<Block['block'], string> = { asset: 'Asset', weight: 'Split', if: 'If / Otherwise', filter: 'Pick' };

type BlockSheetProps = {
  /** The block being edited. Null closes the sheet. */
  block: Block | null;
  actions: BlockActions;
  onChange: (block: Block) => void;
  onClose: () => void;
};

/** The editor of one block, in a sheet: what it can be changed to, and the ways to move or remove it. */
export function BlockSheet({ block, actions, onChange, onClose }: BlockSheetProps) {
  // The sheet keeps drawing the last block while it closes, so it does not go blank on the way out.
  const [last, setLast] = useState<Block | null>(block);
  useEffect(() => {
    if (block) setLast(block);
  }, [block]);
  const shown = block ?? last;
  let inside: ReactNode = null;
  if (shown?.block === 'asset') inside = <AssetButton label="Asset" asset={shown.asset} onChange={(asset) => onChange({ block: 'asset', asset })} />;
  if (shown?.block === 'weight') inside = <SplitEditor block={shown} onChange={onChange} />;
  if (shown?.block === 'if') inside = <IfEditor block={shown} onChange={onChange} />;
  if (shown?.block === 'filter') inside = <PickEditor block={shown} onChange={onChange} />;
  return (
    <Sheet open={block !== null} onClose={onClose} title={shown ? EDITOR_TITLES[shown.block] : ''} actions={<Button variant="primary" fullWidth onClick={onClose}>Done</Button>}>
      {inside}
      <Actions actions={actions} />
    </Sheet>
  );
}

/** The kinds of block there are, each with one plain line. */
export function AddSheet({ open, onPick, onClose }: { open: boolean; onPick: (kind: (typeof BLOCK_KINDS)[number]['kind']) => void; onClose: () => void }) {
  return (
    <Sheet open={open} onClose={onClose} title="Add">
      <ul className="picker-list" role="list">
        {BLOCK_KINDS.map((each) => (
          <li key={each.kind}>
            <button type="button" className="picker-row picker-kind" onClick={() => onPick(each.kind)}>
              <span className={`pill pill-${each.kind === 'weight' ? 'split' : each.kind === 'filter' ? 'pick' : each.kind} pill-label`}>{each.name}</span>
              <span className="picker-name">{each.line}</span>
            </button>
          </li>
        ))}
      </ul>
    </Sheet>
  );
}

const FREQUENCY_OPTIONS = FREQUENCIES.map((value) => ({ value, label: FREQUENCY_WORDS[value] }));

/** When the fund trades: on a calendar, or once a holding has drifted far enough. */
export function TradesSheet({ open, rebalance, onChange, onClose }: { open: boolean; rebalance: Rebalance; onChange: (rebalance: Rebalance) => void; onClose: () => void }) {
  return (
    <Sheet open={open} onClose={onClose} title="When it trades" actions={<Button variant="primary" fullWidth onClick={onClose}>Done</Button>}>
      <div className="editor">
        <Segmented
          label="What sets it off"
          name="trades-type"
          items={[
            { value: 'calendar', label: 'The calendar' },
            { value: 'threshold', label: 'Drift' },
          ]}
          value={rebalance.type}
          onChange={(type) => onChange(type === 'calendar' ? { type, frequency: 'monthly' } : { type, corridor: 0.05 })}
        />
        {rebalance.type === 'calendar' ? (
          <>
            <Choice label="How often" value={rebalance.frequency} options={FREQUENCY_OPTIONS} onChange={(frequency: Frequency) => onChange({ type: 'calendar', frequency })} />
            <p className="form-hint">On that day the fund is put back to what its rules say.</p>
          </>
        ) : (
          <>
            <PercentInput label="Drift" value={rebalance.corridor} max={100} onChange={(corridor) => onChange({ type: 'threshold', corridor })} />
            <p className="form-hint">The fund trades once any holding is this far from its target, and not before.</p>
          </>
        )}
      </div>
    </Sheet>
  );
}
