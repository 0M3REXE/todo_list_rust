-- Users, and the table tower-sessions keeps its session records in.
--
-- `email` is stored exactly as submitted apart from trimming, and uniqueness is
-- decided on lower(email). That makes "Alice@example.com" and
-- "alice@example.com" the same account without rewriting the stored value, so
-- what the user sees is what we kept.
create table users (
    id                bigint generated always as identity primary key,
    email             text not null,
    password_hash     text not null,
    email_verified_at timestamptz,
    created_at        timestamptz not null default now(),

    constraint users_email_length check (char_length(btrim(email)) between 3 and 254)
);

create unique index users_email_key on users (lower(email));

-- Session records for tower-sessions, via the store in src/auth/store.rs.
--
-- `id` is a 128-bit random integer, which overflows bigint, so it is kept as
-- text and converted in Rust. `data` is the whole session record as JSON.
create table sessions (
    id          text primary key,
    data        jsonb not null,
    expiry_date timestamptz not null
);

-- The cleanup task runs "delete from sessions where expiry_date <= now()", so
-- the index has to lead with expiry_date.
create index sessions_expiry_idx on sessions (expiry_date);