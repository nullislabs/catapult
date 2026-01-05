-- PR comments tracking table
-- Tracks the GitHub comment ID for each PR deployment so we can update
-- the same comment on subsequent pushes instead of creating new ones.

CREATE TABLE IF NOT EXISTS pr_comments (
  id SERIAL PRIMARY KEY,
  github_org VARCHAR(255) NOT NULL,
  github_repo VARCHAR(255) NOT NULL,
  pr_number INTEGER NOT NULL,
  comment_id BIGINT NOT NULL,
  created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,

  -- Unique constraint on org/repo/pr_number
  CONSTRAINT pr_comments_unique UNIQUE (github_org, github_repo, pr_number)
);

-- Index for lookups
CREATE INDEX IF NOT EXISTS idx_pr_comments_lookup
  ON pr_comments(LOWER(github_org), LOWER(github_repo), pr_number);
