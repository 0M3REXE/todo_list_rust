-- Password changes, email verification and password reset all need two things
-- the first migration did not have: a way to find every session belonging to an
-- account, and a place to put one-time tokens.

-- `user_id` is copied out of the session payload when the session is written.
--
-- "Log out everywhere" and "reset the password" both mean "throw away every
-- session for this account", which needs to find sessions by owner. Without a
-- column that means reaching into the JSON payload from SQL:
--
--     where data -> 'axum-login.data' ->> 'user_id' = $1
--
-- A real column is easier to read, index, and get right. The copying happens in
-- src/auth/store.rs, which reads the same key constant the auth layer writes.
--
-- Null for sessions that belong to nobody — a visitor who has not logged in.
alter table sessions add column user_id bigint references users (id) on delete cascade;

create index sessions_user_id_idx on sessions (user_id);

-- One-time tokens for email verification and password reset.
--
-- `token_hash` is the SHA-256 of the token, never the token itself. A leaked
-- database must not hand out working reset links, and a hash is useless for
-- that: guessing one costs 2^256.
--
-- `purpose` keeps a verification link from being pasted into the reset form.
-- Single use is enforced by `used_at` being set inside the consuming UPDATE, so
-- two racing requests cannot both win.
create table email_tokens (
    id         bigint generated always as identity primary key,
    user_id    bigint not null references users (id) on delete cascade,
    purpose    text   not null,
    token_hash text   not null,
    expires_at timestamptz not null,
    used_at    timestamptz,
    created_at timestamptz not null default now(),

    constraint email_tokens_purpose check (purpose in ('verify', 'reset'))
);

-- The lookup that matters is "the live token for this user and purpose".
create index email_tokens_lookup_idx on email_tokens (user_id, purpose, expires_at);

-- The expiry sweep runs "delete from email_tokens where expires_at <= now()".
create index email_tokens_expires_idx on email_tokens (expires_at);