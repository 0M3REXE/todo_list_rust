# Project audit and remediation plan

**Audit date:** 2026-10-24  
**Scope:** Rust/Axum application, auth and todo flows, database/schema, HTMX and browser assets, templates/copy, deployment configuration, tests and project documentation.  
**Method:** Static repository review, implementation, focused Rust tests, browser inspection of the public deployment, JavaScript syntax checks, and diff validation. No production data or `.env` values were accessed, and no deployment was changed.

## Implementation status

Completed in the current working tree: htmx 4 protocol updates; atomic user-id return; session creation now retries only actual id collisions and surfaces other database errors; generic duplicate-signup response; Vercel database-backed auth throttling; account-action notices; consistent Unicode todo limits; private response cache headers; HSTS and Vercel configuration checks; PWA asset refresh; keyboard filter/focus handling; corrected README and architecture guides; removal of unused Alpine; and unignoring the existing test suite so it appears in Git status.

Product scope confirmed by the user: email remains the login identifier, but the app will not send email. Email verification and password reset therefore remain out of scope. Redis is also out of scope; Vercel throttling uses the existing libSQL database.

Still outstanding: deploy the working-tree fixes before expecting them on the public URL; run a post-deploy smoke check for Vercel request headers; validate screen-reader/focus behavior; and decide whether account deletion/export or offline task storage belong in the product. The current live deployment has no working `/favicon.ico` route; the working tree now adds it.

## Executive summary

The core todo ownership model is clear: todo queries are parameterized and scoped by account, passwords are Argon2-hashed off the async worker, sessions are HttpOnly, state-changing browser requests have an origin check, and dynamic output is rendered through Askama. The project has a substantial HTTP and model test suite.

At discovery, the most actionable problems were integration drift and incomplete product flows. The implementation fixes below are now in the working tree:

1. **Resolved:** htmx partial-request detection and lifecycle listeners now use htmx 4 names; search state and keyboard focus are restored after swaps.
2. **Resolved:** account creation uses a single `INSERT ... RETURNING id` operation.
3. **Resolved:** password change and logout-everywhere confirmations are shown on the account page.
4. **Resolved:** README and architecture guides now describe libSQL, current routes, and current account capabilities.
5. **Resolved within the existing stack:** fixed-window authentication throttling uses hashed Vercel client IPs and atomic libSQL upserts; it adds no email provider or Redis dependency. Duplicate signup copy is generic. Password recovery remains intentionally unavailable because it would require outbound email.
6. **Resolved:** the todo limit now counts Unicode code points consistently in Rust and client-side validation; the browser no longer applies its different native `maxlength` rule.

Additional fixes in the working tree: private HTML uses `no-store`; HTTPS deployments set HSTS; Vercel startup validates the database URL, secure cookies, and cron secret; static assets use network-first cache updates with offline fallback; unused Alpine was removed; and keyboard users retain focus after todo mutations.

No confirmed critical vulnerability was established by static inspection alone. `cargo test --offline --test verify_http` passes all 17 tests, `cargo check --offline --all-targets` and Node syntax checks for the edited browser scripts pass, and `git diff --check` is clean. Remaining verification is visual/browser accessibility review and a post-deploy smoke check for Vercel request headers and the favicon route.

## Findings and decisions

Priority meanings: **P0** blocks safe operation or risks a severe incident; **P1** should be addressed before the next meaningful release; **P2** is important product/security/quality work; **P3** is cleanup or polish. Product-scope items are marked **Decision** where they need an explicit product choice rather than being objectively broken.

### P1 — Align the htmx 4 integration — addressed

- **Evidence:** `static/vendor/htmx.min.js` identifies itself as 4.0.0. `src/auth/middleware.rs` detects partial requests using `hx-request`. `static/keyboard.js` listens for `htmx:afterSwap`. The project’s own `docs/superpowers/specs/2026-10-08-rust-axum-htmx-curriculum-design.md` says htmx 4 uses `HX-Request-Type` and renamed events; the official htmx 4 migration notes list `htmx:after:swap` as the replacement.
- **Impact:** When an authenticated partial request expires, the guard may take the full-page redirect path instead of its htmx redirect response. Search filtering is not restored after server-rendered list replacements. Other old event listeners or protocol assumptions may be hidden elsewhere.
- **Plan:** Run the htmx 4 upgrade checker; update the partial-request check to the supported request type; update lifecycle listeners; inspect all headers, response swaps, error handling, and `hx-*` assumptions against the installed version. Keep a short compatibility note tied to the vendored version.
- **Completion:** Auth expiry during a partial request navigates cleanly to login; the search filter remains applied after toggle/delete; checker findings are resolved or explicitly waived; behavior is covered at both handler and browser level.

### P1 — Make account creation and its returned identity atomic — addressed

- **Evidence:** `src/auth/store.rs` inserts a user, awaits completion, then runs `select last_insert_rowid()` in a second call.
- **Impact:** `last_insert_rowid()` was connection-scoped state and could couple concurrent inserts. The implementation now uses one `INSERT ... RETURNING id` operation, removing that dependency.
- **Plan:** Use one insert-returning operation if supported by both local and remote libSQL, or serialize the insert-and-id-read as a transaction with documented connection semantics. Avoid relying on process-wide request order.
- **Completion:** Concurrent signup attempts for distinct emails always receive their own account ids; local and Turso paths share the same behavior.

### P1 — Add abuse controls around authentication and expensive work — addressed in app

- **Evidence:** Signup and login call Argon2; there is no limiter in the router or handlers. The `.env.example` explicitly says rate limiting is not yet used. Login failures are uniform, but repeated online guesses remain unbounded. Duplicate signup returns “That email is already registered.”
- **Impact:** Attackers can consume CPU with hashing and attempt password guesses, and can enumerate registered email addresses through signup.
- **Plan:** Choose a limiter strategy suitable for both local and Vercel deployments (shared durable store or platform edge control; in-memory counters alone do not coordinate serverless instances). Limit login, signup, password change, and any future reset/verification endpoints. Define identity keys, proxy trust, limits, retry behavior, and observability. Decide whether signup should disclose duplicate addresses; prefer a uniform response if enumeration resistance is a requirement.
- **Completion:** Limits hold across deployed instances; forwarded client IP is trusted only behind a known proxy; throttling responses are clear and do not expose account existence; operational dashboards can distinguish throttling from application errors.

### P1 — Make account-lifecycle copy match the chosen scope — addressed

- **Evidence:** At audit time `README.md` promised email verification and password reset, but the implementation only supported signup/login/logout/password change/logout-everywhere.
- **Impact:** The published product description is false; users cannot recover a forgotten password or prove control of an address. Invalid addresses can become permanent accounts. If email verification is intended, unverified accounts currently receive full access.
- **Decision and plan:** The user confirmed email remains the login identifier but no provider should send email. Verification and reset are out of scope. Claims were removed, README explains recovery is unavailable, and duplicate-signup copy no longer confirms whether an address exists.
- **Completion:** Public setup and account copy describe the current no-email account flow accurately.

### P2 — Show account-action success and preserve account-page state — addressed

- **Evidence:** `change_password` redirects to `?changed=1` and `logout_everywhere` redirects to `?logged_out=1` in `src/auth/handlers.rs`; `account_form` always supplies `notice: None`. `templates/account_password.html` has a success notice region that is never populated.
- **Impact:** Users receive no confirmation, especially after ending sessions on other devices. The session count can also appear unchanged or confusing immediately after the action.
- **Plan:** Parse a small allowlisted action query or use flash/session messages; set specific copy for each action; reload a live session count after the operation; ensure refresh does not repeat the action.
- **Completion:** Each successful action displays an accurate confirmation once; account counts reflect the persisted session state; errors continue to render safely.

### P2 — Define one Unicode-aware todo length rule — addressed

- **Evidence:** `src/todo/store.rs` uses `text.len()` (UTF-8 bytes); the schema uses SQLite `length(trim(text))` (characters); templates set `maxlength="280"` (browser-defined string units). Password validation already deliberately counts Unicode characters, showing that length semantics matter in this app.
- **Impact:** Emoji and some non-ASCII text can be rejected by the server even when below the advertised/schema character limit. Browser and database acceptance boundaries differ.
- **Plan:** Pick the product rule (Unicode scalar values, grapheme clusters, or bytes), implement the same rule server-side and in persistence, and make the UI’s hint/limit match. If grapheme clusters are chosen, account for composed and decomposed forms and normalization policy.
- **Completion:** Boundary cases have the same result in browser validation, handler validation, and database constraint; copy describes the rule in human terms.

### P2 — Stop persistent caching of private responses and add transport hardening — addressed

- **Evidence:** `src/app.rs::cache_control` uses `private, no-cache, max-age=0`, which permits a browser to retain a private response and revalidate it later. The app sets CSP, nosniff, referrer, and permissions policy, but does not set HSTS. The service worker correctly avoids caching application navigations and todo responses.
- **Impact:** Private task/account pages can remain in browser cache on shared devices or after logout. Missing HSTS leaves first-visit downgrade protection to the hosting platform.
- **Plan:** Use `Cache-Control: no-store` for authenticated HTML and fragments, including error responses; keep explicit cache policies for public versioned assets. Confirm the hosting platform’s TLS/HSTS behavior, then set an appropriate production-only HSTS policy if not already guaranteed. Verify logout and session-expiry flows against browser back/forward cache behavior.
- **Completion:** Authenticated responses are not persisted by normal HTTP caches; public assets retain intentional caching; production sends the agreed HSTS policy without breaking local HTTP development.

### P2 — Validate deployment secrets and operational settings at startup — addressed

- **Evidence:** `src/app.rs` returns 503 when `CRON_SECRET` is absent but accepts any configured value, including an empty or weak value. `README.md` recommends at least 16 characters, but does not enforce it. `SESSION_SECURE` silently defaults true; `DATABASE_URL` silently defaults to a local file.
- **Impact:** Misconfigured production can appear to start while the cleanup endpoint is unusable, or can use unintended local storage. An empty cron secret weakens the maintenance endpoint.
- **Plan:** Add environment-specific configuration validation and fail-fast messages for production: database URL must be explicitly configured; cron secret must be non-empty and meet the selected strength rule; secure cookies must remain enabled; trusted host/proxy settings must be explicit. Keep local defaults convenient and document them separately.
- **Completion:** Invalid production configuration fails on cold start with a safe diagnostic; local startup remains documented; deployment settings are reproducible from a checked-in example without secrets.

### P2 — Make the product promise and navigation copy accurate — addressed in docs

- **Evidence:** `README.md` describes Postgres, email verification, and password reset. Current code uses libSQL/Turso. `templates/landing.html` says “Your list is private — tied to your account” and “works on any device”; no privacy/data-retention policy or account recovery flow is documented. The README’s setup steps focus on CSS and omit the current database/session configuration.
- **Impact:** Contributors may follow incorrect setup instructions; users may rely on privacy, portability, verification, or recovery guarantees that have not been defined.
- **Plan:** Refresh README setup, architecture, and deployment instructions from current code. Review every landing/auth/account claim against implemented behavior. Add concise privacy/data retention, backup/export, and account-recovery explanations once product decisions are made. Ensure page title, app name, footer year, and manifest branding come from one source or are updated together.
- **Completion:** A new contributor can run the app from a clean checkout using only README instructions; all public claims are true and supportable.

### P2 — Review CSRF, host trust, cookies, and error behavior as one deployment boundary — origin check addressed; deployment check pending

- **Evidence:** The original independent `Sec-Fetch-Site: cross-site` rejection could reject a request even when its Origin matched the app. The guard now gives a valid Origin/Referer precedence, fails closed for a browser request marked cross-site when no origin proof is supplied, and compares exact normalized hosts. Vercel documents that `Host` carries the public hostname, including custom domains. Cookies are HttpOnly and SameSite=Lax. `AppError` returns generic 500 text; session error strings remain logged.
- **Impact:** Deployment proxies and caller-controlled host forwarding are security-sensitive. CSRF behavior is under-documented for the production Vercel boundary; logs should not accidentally contain sensitive session data if library errors expand.
- **Plan:** Keep Host as the deployment boundary and avoid caller-controlled forwarding fallbacks. Focused HTTP tests now cover trusted same-origin, custom host, foreign origin, malformed referer parsing, cross-site metadata, and missing origin proof. Review logs for cookies, credentials, hashes, tokens, and request bodies. Keep secrets out of tracing fields.
- **Completion:** A written deployment threat model matches actual Vercel behavior; origin/host checks have a defined allowlist; logs contain no credentials or session identifiers.

### P2 — Complete keyboard, focus, and assistive-technology behavior — partly addressed

- **Evidence:** `templates/todo_list.html` replaces the entire list after toggle/delete, which can remove the focused control. `static/keyboard.js` has a search shortcut `/` and new-task shortcut `n`; the latter is suppressed while typing but there is no visible shortcut/help affordance. The script does not honor modifier keys for `/` consistently and has no reduced-motion relevance because it is interaction-focused. Search status is not announced.
- **Impact:** Focus can be lost after ordinary actions; shortcut behavior may surprise users; filtered-result changes are not clearly announced to screen readers.
- **Plan:** Reapply filter state after swaps; restore focus to the corresponding control or a stable list heading; expose keyboard shortcuts accessibly and avoid intercepting browser/assistive shortcuts; announce no-match/result counts with a polite live region; verify contrast, zoom, touch target size, and visible focus styles in both themes.
- **Completion:** Todo actions remain keyboard-operable without focus loss; search/filter changes are announced; all primary flows pass manual WCAG-oriented checks at narrow and zoomed layouts.

### P2 — Resolve PWA cache versioning and offline expectations — code addressed, browser check pending

- **Evidence:** `static/service-worker.js` uses a manually named cache (`todo-v5`) with cache-first handling for static paths. The root `service-worker.js` is served `no-cache`, but app.css and some assets use stable URLs. Cache cleanup only occurs when the cache name changes.
- **Impact:** A changed service worker can reuse an existing named cache containing stale assets. Users may see old CSS/scripts after deployment. Offline behavior is read-only and should not imply tasks are available offline.
- **Plan:** Establish an asset versioning rule: content-hashed URLs or a build-generated cache revision, and a deliberate cache migration. Retain the privacy rule that account pages and todo responses are never cached. Make offline copy and install prompts state that task data requires a connection.
- **Completion:** A new deployment reliably updates cached assets; offline fallback remains data-free; browser install/start routes and cache cleanup are exercised in a real PWA-capable browser.

### P3 — Remove stale guides and dead assets; improve generated CSS maintenance — partly addressed

- **Evidence:** `README.md` and `docs/architecture.md` include obsolete Postgres/SQLx and older htmx descriptions. `.gitignore` ignores `/docs`, while the repository contains extensive documentation there, so those documents are local-only unless intentionally force-added. `static/vendor/alpine.min.js` appears unreferenced by templates/routes; `static/site.css` is used by `static/tailwind.css`, so it is not dead. The built `static/app.css` is embedded in `src/app.rs` and requires manual regeneration.
- **Impact:** Stale documentation causes incorrect operations and code changes; ignored documentation may disappear for other contributors; unused vendored code and manual asset synchronization create drift.
- **Plan:** Decide which docs belong in version control and align `.gitignore`. Rewrite or archive obsolete guides; do not mechanically update historical specs. Confirm whether Alpine is needed; remove it if unused. Add a lightweight documented or automated check that generated CSS is current after template/style changes, and ensure all assets served by the app are intentional.
- **Completion:** Tracked docs describe the current system; no user-facing setup guide contradicts the code; unused vendor files are removed or referenced; generated CSS differences are detectable in review.

## Remediation sequence

### Phase 0 — Confirm scope and contain the highest-risk drift

1. Decide if email verification and password reset are product requirements or stale copy.
2. Run the htmx 4 upgrade checker and inventory old protocol/event usage.
3. **Done:** replace the two-step user insert/id lookup with one atomic `INSERT ... RETURNING id` operation.
4. Decide the production rate-limiting mechanism and the Vercel trusted-proxy contract.
5. Add an issue register for anything that needs product, hosting, or legal input.

**Exit gate:** No release-blocking htmx or identity-assignment ambiguity remains; decisions for email/recovery and rate limiting are recorded.

### Phase 1 — Security and account reliability

1. Implement shared rate limiting for login/signup and expensive password operations.
2. Remove duplicate-email disclosure or explicitly accept it as a UX tradeoff; add limits either way.
3. Validate production configuration, including database, cookie security, cron secret, and trusted domains.
4. Set no-store on private responses and confirm HSTS/TLS behavior.
5. Exercise CSRF/origin behavior through the real deployment proxy/domain setup.
6. If recovery is in scope, implement email verification and password reset with expiring, one-time tokens and uniform request responses.

**Exit gate:** Auth endpoints have bounded resource consumption; account lifecycle and deployment secrets have explicit, tested behavior; private responses and logging are reviewed.

### Phase 2 — Correctness and UX

1. Make account creation return its identity atomically.
2. Correct and display account action notices and session counts.
3. Standardize todo length semantics across client, Rust, and database.
4. Update htmx headers/events, preserve search state, restore focus, and announce filter outcomes.
5. Validate signup email enough to prevent unusable addresses; align validation with the chosen verification/recovery flow.
6. Confirm the installed PWA updates static assets after deployment and keeps private data out of caches.

**Exit gate:** Common account and todo flows behave consistently for Unicode, expired sessions, filtered lists, keyboard users, and installed PWA users.

### Phase 3 — Product decisions and copy

1. Decide whether the product needs edit, undo, bulk clear, task ordering, export, or account deletion. Treat these as roadmap choices, not presumed defects.
2. Define backup, retention, privacy, support, and account-recovery expectations before making public promises.
3. Rewrite landing, signup, offline, and account copy against shipped capabilities.
4. Harmonize the product name, app title, manifest labels, icon assets, and footer year.

**Exit gate:** Product claims are accurate, recovery expectations are explicit, and the next product investments have an owner and user-value rationale.

### Phase 4 — Documentation, cleanup, and release readiness

1. Refresh README and architecture guide for libSQL/Turso, current routes, current session design, and current htmx version.
2. Decide whether `/docs` should be tracked; change ignore rules accordingly and identify authoritative docs.
3. Remove unreferenced Alpine assets if confirmed unused; retain `site.css` because Tailwind imports it.
4. Add generated CSS drift detection and record the vendored htmx version/update process.
5. Run the existing Rust suite and add only the missing coverage identified by findings: concurrent signup identity, rate limits, htmx 4 request handling/events, success notices, Unicode boundary, private cache headers, cron secret validation, and browser focus/PWA behavior.
6. Perform a deployment smoke check on preview and production hostnames, then review response headers, logs, cron invocation, and database configuration.

**Exit gate:** Clean-checkout setup works; documentation and deployment variables agree with code; existing and added checks pass; preview smoke checks match production assumptions.

## Product decisions to record

- Is email verification required before a user can store tasks?
- Is password reset required for launch, and which email provider is supported?
- Should signup reveal that an email already exists, or use a uniform response?
- Is offline task editing intentionally out of scope, or should the app become genuinely offline-capable?
- Are export and account deletion required for user trust/data-retention policy?
- What is the intended todo limit in user-visible characters: Unicode code points, grapheme clusters, or bytes?
- Is this a learning project or a public production service? That changes the acceptable operational burden for email delivery, shared rate limiting, monitoring, and backup guarantees.

## Review limits

The audit included a read-only browser check of the public landing page and its declared icon. Navigating to the deployed `/favicon.ico` currently fails, which the working-tree route fixes after deployment. The focused HTTP suite passes (17 tests), along with `cargo check --offline --all-targets`, JavaScript syntax checks, and `git diff --check`. I did not inspect `.env` values or production data, and did not deploy changes or verify request headers from a post-deploy production invocation.

## Sources

- Project implementation and tests in `src/`, `templates/`, `static/`, `migrations/`, and `tests/`.
- Project notes: `docs/superpowers/specs/2026-10-08-rust-axum-htmx-curriculum-design.md`, `docs/architecture.md`, `.env.example`, and `README.md`.
- [Official htmx 4 release and migration notes](https://four.htmx.org/announcements/2026-08-28-htmx-4.0.0-is-released) and [htmx 4 request/event documentation](https://four.htmx.org/docs/whats-new-in-htmx-4).
- [Vercel request headers](https://vercel.com/docs/headers/request-headers) for the public `Host` and deployment-provided client IP headers.
