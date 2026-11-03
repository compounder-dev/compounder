//! Sign in with an email and a password. An account is opened with the invite
//! code in `INVITE_CODE`. A session is a random token in a cookie, good for 30
//! days. Accounts and sessions are kept in one file in `AUTH_DIR`.
//!
//! With no `AUTH_DIR` the app is open, as it is on a laptop and in tests.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{
    password_hash::{rand_core::OsRng, SaltString},
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
};
use axum::{
    extract::{Form, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;

const COOKIE: &str = "session";

/// Who a signed-in request is from: their email.
#[derive(Clone, Debug)]
pub struct User(pub String);
const SESSION_SECONDS: u64 = 30 * 24 * 3600;

#[derive(Default, Serialize, Deserialize)]
struct Store {
    /// Email to password hash.
    users: BTreeMap<String, String>,
    /// Token to session.
    sessions: BTreeMap<String, Session>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    email: String,
    expires: u64,
}

#[derive(Clone)]
pub struct Auth {
    file: PathBuf,
    store: Arc<Mutex<Store>>,
    invite: Option<String>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

impl Auth {
    /// Sign-in is on when `AUTH_DIR` names a folder.
    pub fn from_env() -> Option<Auth> {
        let dir = PathBuf::from(std::env::var_os("AUTH_DIR")?);
        let invite = std::env::var("INVITE_CODE").ok().filter(|code| !code.is_empty());
        Some(Auth::at(dir, invite))
    }

    pub fn at(dir: PathBuf, invite: Option<String>) -> Auth {
        let file = dir.join("auth.json");
        let store = std::fs::read(&file).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Auth { file, store: Arc::new(Mutex::new(store)), invite }
    }

    fn save(&self, store: &Store) -> std::io::Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.file.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(store)?)?;
        std::fs::rename(tmp, &self.file)
    }

    /// The email a live session belongs to.
    fn signed_in(&self, token: &str) -> Option<String> {
        let store = self.store.lock().unwrap();
        store.sessions.get(token).filter(|session| session.expires > now()).map(|session| session.email.clone())
    }

    fn start(&self, store: &mut Store, email: &str) -> String {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let time = now();
        store.sessions.retain(|_, session| session.expires > time);
        store.sessions.insert(token.clone(), Session { email: email.to_string(), expires: time + SESSION_SECONDS });
        token
    }
}

fn token_of(request: &Request) -> Option<String> {
    let cookies = request.headers().get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| pair.trim().strip_prefix(&format!("{COOKIE}=")).map(str::to_string))
}

fn session_cookie(token: &str, seconds: u64) -> HeaderValue {
    HeaderValue::from_str(&format!("{COOKIE}={token}; Path=/; Max-Age={seconds}; HttpOnly; Secure; SameSite=Lax"))
        .expect("a cookie is plain text")
}

/// Only a path inside this app may be gone to after signing in.
fn local(next: Option<&str>) -> String {
    match next {
        Some(path) if path.starts_with('/') && !path.starts_with("//") && !path.starts_with("/\\") => path.to_string(),
        _ => "/".to_string(),
    }
}

/// Every request but the sign-in pages, the health check and the files the
/// build put beside the shell needs a session. A page sends you to sign in; the
/// API answers 401.
pub async fn gate(State(auth): State<Auth>, mut request: Request, next: Next) -> Response {
    if let Some(email) = token_of(&request).and_then(|token| auth.signed_in(&token)) {
        request.extensions_mut().insert(User(email));
        return next.run(request).await;
    }
    let path = request.uri().path();
    let open = matches!(path, "/login" | "/signup" | "/logout" | "/health")
        || path.starts_with("/assets/")
        || path.rsplit('/').next().is_some_and(|last| last.contains('.'));
    if open {
        return next.run(request).await;
    }
    if path.starts_with("/v1/") {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": { "code": "signed_out", "message": "sign in first" } }))).into_response();
    }
    let here = request.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let next: String = url_encode(here);
    Redirect::to(&format!("/login?next={next}")).into_response()
}

fn url_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
struct Next_ {
    next: Option<String>,
}

#[derive(Deserialize)]
struct Invite {
    invite: Option<String>,
}

#[derive(Deserialize)]
struct SignIn {
    email: String,
    password: String,
    next: Option<String>,
}

#[derive(Deserialize)]
struct SignUp {
    invite: String,
    email: String,
    password: String,
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

/// A page with one form: the title, an error if there is one, the fields and one button.
fn form_page(title: &str, action: &str, error: Option<&str>, hidden: &[(&str, &str)], button: &str, new_password: bool) -> Html<String> {
    let hidden: String =
        hidden.iter().map(|(name, value)| format!(r#"<input type="hidden" name="{name}" value="{}">"#, escape(value))).collect();
    let error = error.map(|e| format!(r#"<p class="error" role="alert">{}</p>"#, escape(e))).unwrap_or_default();
    let password_use = if new_password { "new-password" } else { "current-password" };
    Html(format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="theme-color" content="#111111"><title>{title} · Compounder</title><style>
:root{{color-scheme:light dark;--ink:#111;--ground:#fafafa;--field:#fff;--line:#d9d9d9;--muted:#666;--accent:#111;--on-accent:#fff;--loss:#c62828}}
@media (prefers-color-scheme:dark){{:root{{--ink:#f5f5f5;--ground:#111;--field:#1c1c1c;--line:#333;--muted:#aaa;--accent:#f5f5f5;--on-accent:#111;--loss:#ff7b72}}}}
*{{box-sizing:border-box}}body{{margin:0;font:16px/24px system-ui,-apple-system,sans-serif;background:var(--ground);color:var(--ink)}}
main{{max-width:400px;margin:0 auto;padding:64px 24px}}h1{{font-weight:500;font-size:32px;line-height:40px;margin:0 0 32px}}
label{{display:block;font-size:14px;color:var(--muted);margin:16px 0 4px}}
input[type=email],input[type=password]{{width:100%;font:inherit;padding:12px 14px;border:1px solid var(--line);border-radius:12px;background:var(--field);color:var(--ink)}}
button{{width:100%;margin-top:32px;font:inherit;font-weight:500;padding:14px;border:0;border-radius:999px;background:var(--accent);color:var(--on-accent)}}
.error{{color:var(--loss);margin:0 0 8px}}</style></head><body><main><h1>{title}</h1>{error}
<form method="post" action="{action}">{hidden}<label for="email">Email</label><input id="email" name="email" type="email" autocomplete="email" required autofocus>
<label for="password">Password</label><input id="password" name="password" type="password" autocomplete="{password_use}" required minlength="8">
<button type="submit">{button}</button></form></main></body></html>"##
    ))
}

async fn login_page(Query(query): Query<Next_>) -> Html<String> {
    form_page("Sign in", "/login", None, &[("next", &local(query.next.as_deref()))], "Sign in", false)
}

async fn login(State(auth): State<Auth>, Form(form): Form<SignIn>) -> Response {
    let email = form.email.trim().to_lowercase();
    let next = local(form.next.as_deref());
    let hash = auth.store.lock().unwrap().users.get(&email).cloned();
    let ok = tokio::task::spawn_blocking(move || {
        hash.is_some_and(|hash| {
            PasswordHash::new(&hash).is_ok_and(|parsed| Argon2::default().verify_password(form.password.as_bytes(), &parsed).is_ok())
        })
    })
    .await
    .unwrap_or(false);
    if !ok {
        let page = form_page("Sign in", "/login", Some("Wrong email or password."), &[("next", &next)], "Sign in", false);
        return (StatusCode::UNAUTHORIZED, page).into_response();
    }
    let mut store = auth.store.lock().unwrap();
    let token = auth.start(&mut store, &email);
    if auth.save(&store).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "could not save the session").into_response();
    }
    let mut response = Redirect::to(&next).into_response();
    response.headers_mut().insert(header::SET_COOKIE, session_cookie(&token, SESSION_SECONDS));
    response
}

fn invite_ok(auth: &Auth, code: Option<&str>) -> bool {
    matches!((&auth.invite, code), (Some(invite), Some(code)) if invite == code)
}

async fn signup_page(State(auth): State<Auth>, Query(query): Query<Invite>) -> Response {
    if !invite_ok(&auth, query.invite.as_deref()) {
        return (StatusCode::NOT_FOUND, "This invite is not valid.").into_response();
    }
    form_page("Create account", "/signup", None, &[("invite", query.invite.as_deref().unwrap_or(""))], "Create account", true)
        .into_response()
}

async fn signup(State(auth): State<Auth>, Form(form): Form<SignUp>) -> Response {
    if !invite_ok(&auth, Some(&form.invite)) {
        return (StatusCode::NOT_FOUND, "This invite is not valid.").into_response();
    }
    let email = form.email.trim().to_lowercase();
    let again = |error: &str| {
        (StatusCode::UNPROCESSABLE_ENTITY, form_page("Create account", "/signup", Some(error), &[("invite", &form.invite)], "Create account", true))
            .into_response()
    };
    if !email.contains('@') {
        return again("That is not an email address.");
    }
    if form.password.chars().count() < 8 {
        return again("Use at least 8 characters.");
    }
    if auth.store.lock().unwrap().users.contains_key(&email) {
        return again("That email already has an account.");
    }
    let password = form.password.clone();
    let Ok(Ok(hash)) = tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default().hash_password(password.as_bytes(), &salt).map(|hash| hash.to_string())
    })
    .await
    else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "could not make the account").into_response();
    };
    let mut store = auth.store.lock().unwrap();
    store.users.insert(email.clone(), hash);
    let token = auth.start(&mut store, &email);
    if auth.save(&store).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "could not save the account").into_response();
    }
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(header::SET_COOKIE, session_cookie(&token, SESSION_SECONDS));
    response
}

async fn logout(State(auth): State<Auth>, request: Request) -> Response {
    if let Some(token) = token_of(&request) {
        let mut store = auth.store.lock().unwrap();
        store.sessions.remove(&token);
        let _ = auth.save(&store);
    }
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(header::SET_COOKIE, session_cookie("", 0));
    response
}

pub fn routes(auth: Auth) -> Router {
    Router::new()
        .route("/login", get(login_page).post(login))
        .route("/signup", get(signup_page).post(signup))
        .route("/logout", post(logout).get(logout))
        .with_state(auth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest, routing::get as route_get};
    use tower::ServiceExt;

    fn app(auth: Auth) -> Router {
        Router::new()
            .route("/live", route_get(|| async { "board" }))
            .route("/v1/live", route_get(|| async { "{}" }))
            .merge(routes(auth.clone()))
            .layer(axum::middleware::from_fn_with_state(auth, gate))
    }

    fn form(path: &str, body: &str, cookie: Option<&str>) -> HttpRequest<Body> {
        let mut request = HttpRequest::post(path).header("content-type", "application/x-www-form-urlencoded");
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        request.body(Body::from(body.to_string())).unwrap()
    }

    fn cookie_of(response: &Response) -> String {
        response.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_string()
    }

    #[tokio::test]
    async fn signed_out_is_sent_to_sign_in_and_an_invite_opens_an_account_that_signs_in() {
        let dir = std::env::temp_dir().join(format!("auth-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let auth = Auth::at(dir.clone(), Some("let-me-in".into()));

        let page = app(auth.clone()).oneshot(HttpRequest::get("/live").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(page.status(), StatusCode::SEE_OTHER);
        assert_eq!(page.headers()[header::LOCATION], "/login?next=/live");
        let api = app(auth.clone()).oneshot(HttpRequest::get("/v1/live").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(api.status(), StatusCode::UNAUTHORIZED);

        let refused = app(auth.clone()).oneshot(form("/signup", "invite=nope&email=a%40b.co&password=longenough", None)).await.unwrap();
        assert_eq!(refused.status(), StatusCode::NOT_FOUND);
        let made = app(auth.clone()).oneshot(form("/signup", "invite=let-me-in&email=A%40b.co&password=longenough", None)).await.unwrap();
        assert_eq!(made.status(), StatusCode::SEE_OTHER);

        let wrong = app(auth.clone()).oneshot(form("/login", "email=a%40b.co&password=wrongwrong", None)).await.unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        let fresh = Auth::at(dir.clone(), None);
        let right = app(fresh.clone()).oneshot(form("/login", "email=a%40b.co&password=longenough&next=//evil.com", None)).await.unwrap();
        assert_eq!(right.headers()[header::LOCATION], "/");
        let cookie = cookie_of(&right);
        let board =
            app(fresh).oneshot(HttpRequest::get("/live").header("cookie", cookie).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(board.status(), StatusCode::OK);
        let _ = std::fs::remove_dir_all(dir);
    }
}
