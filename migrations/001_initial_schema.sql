-- Catapult Database Schema
-- Central + Worker deployment automation system

-- Workers table
-- Registered worker endpoints for each environment
CREATE TABLE IF NOT EXISTS workers (
  id SERIAL PRIMARY KEY,
  environment VARCHAR(50) NOT NULL UNIQUE,  -- 'nullislabs', 'nullispl', etc.
  endpoint VARCHAR(255) NOT NULL,           -- 'https://deployer.nullislabs.io'
  enabled BOOLEAN DEFAULT TRUE,
  last_seen TIMESTAMP,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Index for looking up workers by environment
CREATE INDEX IF NOT EXISTS idx_workers_environment
  ON workers(environment) WHERE enabled = TRUE;

-- Main deployment configuration table
-- Maps GitHub repositories to deployment environments and settings
CREATE TABLE IF NOT EXISTS deployment_config (
  id SERIAL PRIMARY KEY,

  -- GitHub repository identification
  github_org VARCHAR(255) NOT NULL,
  github_repo VARCHAR(255) NOT NULL,

  -- GitHub App installation (cached from webhook for API access)
  installation_id BIGINT,

  -- Deployment target
  environment VARCHAR(50) NOT NULL REFERENCES workers(environment),
  domain VARCHAR(255) NOT NULL,       -- Base domain (e.g., 'example.com')
  subdomain VARCHAR(255),             -- Subdomain for main branch (nullable, e.g., 'www')

  -- Build configuration
  site_type VARCHAR(50) DEFAULT 'auto',  -- 'sveltekit', 'vite', 'zola', 'auto', 'custom'

  -- Status and metadata
  enabled BOOLEAN DEFAULT TRUE,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,

  -- Ensure one config per repo
  UNIQUE(github_org, github_repo)
);

-- Index for fast lookups by org/repo
CREATE INDEX IF NOT EXISTS idx_deployment_config_repo
  ON deployment_config(github_org, github_repo);

-- Index for filtering by environment
CREATE INDEX IF NOT EXISTS idx_deployment_config_env
  ON deployment_config(environment);

-- Index for active deployments only
CREATE INDEX IF NOT EXISTS idx_deployment_config_enabled
  ON deployment_config(enabled) WHERE enabled = TRUE;

-- Deployment history table (tracks all deployment attempts)
CREATE TABLE IF NOT EXISTS deployment_history (
  id SERIAL PRIMARY KEY,
  config_id INTEGER REFERENCES deployment_config(id) ON DELETE CASCADE,

  -- Job tracking (for correlating worker status updates)
  job_id UUID UNIQUE,

  -- Deployment details
  deployment_type VARCHAR(10) NOT NULL,  -- 'pr' or 'main'
  pr_number INTEGER,                     -- NULL for main branch deployments
  branch VARCHAR(255) NOT NULL,
  commit_sha VARCHAR(40) NOT NULL,

  -- Status tracking
  -- Values: 'pending', 'building', 'success', 'failed', 'cleaned'
  status VARCHAR(20) NOT NULL DEFAULT 'pending',
  started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  completed_at TIMESTAMP,

  -- Deployment result
  deployed_url TEXT,
  error_message TEXT,

  -- GitHub integration
  github_comment_id BIGINT              -- ID of GitHub comment for updates
);

-- Index for looking up deployments by job_id (for worker status updates)
CREATE INDEX IF NOT EXISTS idx_deployment_history_job_id
  ON deployment_history(job_id) WHERE job_id IS NOT NULL;

-- Index for finding PR deployments
CREATE INDEX IF NOT EXISTS idx_deployment_history_pr
  ON deployment_history(config_id, pr_number) WHERE pr_number IS NOT NULL;

-- Index for recent deployments
CREATE INDEX IF NOT EXISTS idx_deployment_history_recent
  ON deployment_history(started_at DESC);

-- Index for finding active PR deployments (for cleanup)
CREATE INDEX IF NOT EXISTS idx_deployment_history_active_pr
  ON deployment_history(config_id, pr_number, status)
  WHERE deployment_type = 'pr' AND status = 'success';

-- Function to update updated_at timestamp
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
  NEW.updated_at = CURRENT_TIMESTAMP;
  RETURN NEW;
END;
$$ language 'plpgsql';

-- Triggers to auto-update updated_at
DROP TRIGGER IF EXISTS update_deployment_config_updated_at ON deployment_config;
CREATE TRIGGER update_deployment_config_updated_at
  BEFORE UPDATE ON deployment_config
  FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

DROP TRIGGER IF EXISTS update_workers_updated_at ON workers;
CREATE TRIGGER update_workers_updated_at
  BEFORE UPDATE ON workers
  FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
