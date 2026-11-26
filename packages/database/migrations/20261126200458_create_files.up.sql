-- create_files: the `file` domain, project files and chat attachments with their bytes
-- (docs/design/copilot.md, Data Model). Columns follow .ai/rules/postgres-column-alignment.md.

-- A file's metadata. It belongs to exactly one project or one chat. `name` is for display
-- only, never a path. `page_count` is set for PDFs only.
CREATE TABLE files (
  created_at     timestamptz NOT NULL,
  size_bytes     bigint      NOT NULL CHECK (size_bytes >= 0),
  token_estimate integer     NOT NULL CHECK (token_estimate >= 0),
  page_count     integer     CHECK (page_count >= 0),
  id             uuid        PRIMARY KEY,
  user_id        uuid        NOT NULL,
  project_id     uuid,
  chat_id        uuid,
  included       boolean     NOT NULL DEFAULT true,
  name           text        NOT NULL,
  media_type     text        NOT NULL,
  sha256         bytea       NOT NULL CHECK (octet_length(sha256) = 32),
  CHECK (num_nonnulls(project_id, chat_id) = 1),
  FOREIGN KEY (project_id, user_id) REFERENCES projects (id, user_id) ON DELETE CASCADE,
  FOREIGN KEY (chat_id, user_id) REFERENCES chats (id, user_id) ON DELETE CASCADE
);

CREATE INDEX files_user_id_idx ON files (user_id);
CREATE INDEX files_project_id_idx ON files (project_id);
CREATE INDEX files_chat_id_idx ON files (chat_id);

-- The bytes, read apart from the metadata.
CREATE TABLE file_contents (
  file_id uuid  PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
  bytes   bytea NOT NULL
);

-- Stored out of line without compression: most uploads (images, PDFs, audio) are already
-- compressed.
ALTER TABLE file_contents ALTER COLUMN bytes SET STORAGE EXTERNAL;
