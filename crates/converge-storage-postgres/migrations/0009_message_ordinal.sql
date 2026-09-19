-- A turn's position in the conversation it came from, as the source
-- numbers it: the index in a transcript, or the order an export gives.
-- Two things follow. The same turn sent twice is one message, so a hook
-- can record the exchange a decision cites and the background drain can
-- send it again without minding. And reads order by it, so turns that
-- arrive out of order still read in order — which is what lets the two
-- writers work without coordinating.
--
-- Null for anything recorded before this and for writers that do not
-- number their turns; those keep ordering by `seq`, which for the CLI's
-- own sessions counted the same way.
alter table messages add column ordinal integer;
create unique index messages_session_ordinal
    on messages (session_id, ordinal)
    where ordinal is not null;
