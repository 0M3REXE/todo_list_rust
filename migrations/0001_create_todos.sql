-- The only table in the app.
--
-- id is an identity column: Postgres hands out ids from a sequence, so they
-- are never reused and gaps are normal. The CHECK mirrors the handler's own
-- validation, measured after trimming, so the rule holds even if a future
-- code path forgets the Rust check.
create table todos (
    id   bigint generated always as identity primary key,
    text text not null,
    done boolean not null default false,

    constraint todos_text_length check (char_length(btrim(text)) between 1 and 280)
);

-- Pagination reads ORDER BY done, id. Without this the planner sorts every row
-- on each page request; with it the same rows come back pre-sorted.
create index todos_done_id_idx on todos (done, id);