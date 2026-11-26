-- create_chats: revert everything the .up.sql file does.
DROP TABLE turn_contexts;
DROP TABLE messages;
DROP TABLE chat_tool_approvals;
DROP TABLE chat_tool_servers;
ALTER TABLE schedules DROP CONSTRAINT schedules_source_chat_id_fkey;
DROP TABLE chats;
