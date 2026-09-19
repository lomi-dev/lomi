BEGIN IMMEDIATE;
CREATE TABLE conversations (
  id TEXT PRIMARY KEY, title TEXT NOT NULL, origin TEXT NOT NULL,
  config TEXT NOT NULL, revision INTEGER NOT NULL DEFAULT 0,
  active_leaf TEXT, updated_at INTEGER NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0 CHECK(pinned IN (0,1))
);
CREATE INDEX conversation_history_order ON conversations(pinned DESC,updated_at DESC,id);
CREATE TABLE messages (
  id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  parent_id TEXT, role TEXT NOT NULL CHECK(role IN ('user','assistant')),
  parts_version INTEGER NOT NULL DEFAULT 1, parts TEXT NOT NULL,
  status TEXT NOT NULL, metadata TEXT NOT NULL DEFAULT '{}',
  UNIQUE(conversation_id,id),
  FOREIGN KEY(conversation_id,parent_id) REFERENCES messages(conversation_id,id)
);
CREATE INDEX message_parent ON messages(conversation_id,parent_id);
CREATE TABLE requests (
  id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES messages(id), assistant_id TEXT NOT NULL REFERENCES messages(id),
  fingerprint TEXT NOT NULL, status TEXT NOT NULL, draft_revision INTEGER NOT NULL,
  sequence INTEGER NOT NULL DEFAULT 0, result TEXT NOT NULL DEFAULT '{}'
);
CREATE UNIQUE INDEX one_active_request ON requests(conversation_id) WHERE status='active';
CREATE TABLE drafts (
  conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
  text TEXT NOT NULL DEFAULT '', revision INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE attachments (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, mime TEXT NOT NULL, size INTEGER NOT NULL,
  hash TEXT NOT NULL, object TEXT NOT NULL UNIQUE
);
CREATE TABLE message_attachments (
  message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  attachment_id TEXT NOT NULL REFERENCES attachments(id), ordinal INTEGER NOT NULL,
  PRIMARY KEY(message_id,attachment_id)
);
CREATE TABLE draft_attachments (
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  attachment_id TEXT NOT NULL REFERENCES attachments(id), ordinal INTEGER NOT NULL,
  PRIMARY KEY(conversation_id,attachment_id)
);
CREATE VIRTUAL TABLE chat_search USING fts5(content,conversation UNINDEXED,message UNINDEXED);
PRAGMA user_version=2;
COMMIT;
