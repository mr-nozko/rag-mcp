-- Migration 008: R2 object manifest for bandwidth-aware hydrate
-- Tracks remote etag/size so unchanged docs are not re-downloaded on ephemeral disks.

CREATE TABLE IF NOT EXISTS r2_object_manifest (
  object_key TEXT PRIMARY KEY,
  size INTEGER NOT NULL,
  etag TEXT NOT NULL
);
