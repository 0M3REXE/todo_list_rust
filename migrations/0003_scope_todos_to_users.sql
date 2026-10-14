-- Todos become per-user: every todo now belongs to exactly one account.
--
-- WARNING: the DELETE below throws away every todo currently in the table.
-- This is a development database and those rows have no account to belong to,
-- so there is nothing sensible to migrate them to. Do not run this against a
-- database with real data.
delete from todos;

-- ON DELETE CASCADE means removing a user removes their todos too, so a
-- leftover row can never outlive the account that owned it.
alter table todos add column user_id bigint not null references users (id) on delete cascade;

-- Every query filters by user_id before anything else, so the index has to
-- lead with it or Postgres sorts the whole table for each page load.
drop index todos_done_id_idx;
create index todos_user_done_id_idx on todos (user_id, done, id);