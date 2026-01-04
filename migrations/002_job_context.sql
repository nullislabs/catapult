-- Job context table for tracking deployments
-- This is a simplified table that stores the minimum info needed
-- to update GitHub comments when status updates arrive from workers.

CREATE TABLE IF NOT EXISTS job_context (
  job_id UUID PRIMARY KEY,
  installation_id BIGINT NOT NULL,
  github_org VARCHAR(255) NOT NULL,
  github_repo VARCHAR(255) NOT NULL,
  github_comment_id BIGINT,
  commit_sha VARCHAR(40) NOT NULL,
  created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- Index for cleanup of old records
CREATE INDEX IF NOT EXISTS idx_job_context_created_at
  ON job_context(created_at);

-- Keep records for 7 days, then they can be cleaned up
-- (Manual cleanup or a scheduled job)
