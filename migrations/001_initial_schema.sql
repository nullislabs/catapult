-- Catapult Database Schema
-- Central + Worker deployment automation system
-- Configuration comes from .deploy.json in repos, not from database

-- Workers table
-- Registered worker endpoints for each environment (zone)
CREATE TABLE IF NOT EXISTS workers (
  id SERIAL PRIMARY KEY,
  environment VARCHAR(50) NOT NULL UNIQUE,  -- 'nullislabs', 'nullispl', etc.
  endpoint VARCHAR(255) NOT NULL,           -- 'https://deployer.nullislabs.io'
  enabled BOOLEAN DEFAULT TRUE,
  last_seen TIMESTAMPTZ,
  created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- Index for looking up workers by environment
CREATE INDEX IF NOT EXISTS idx_workers_environment
  ON workers(environment) WHERE enabled = TRUE;

-- Function to update updated_at timestamp
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
  NEW.updated_at = CURRENT_TIMESTAMP;
  RETURN NEW;
END;
$$ language 'plpgsql';

-- Triggers to auto-update updated_at
DROP TRIGGER IF EXISTS update_workers_updated_at ON workers;
CREATE TRIGGER update_workers_updated_at
  BEFORE UPDATE ON workers
  FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Authorized organizations table
-- Controls which GitHub orgs can deploy to which zones and domains
CREATE TABLE IF NOT EXISTS authorized_orgs (
  id SERIAL PRIMARY KEY,
  github_org VARCHAR(255) NOT NULL UNIQUE,  -- GitHub org name (case-insensitive)
  zones TEXT[] NOT NULL DEFAULT '{}',       -- Allowed zones (workers)
  domain_patterns TEXT[] NOT NULL DEFAULT '{}',  -- Allowed domain patterns (e.g., '*.example.com')
  enabled BOOLEAN DEFAULT TRUE,
  created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- Index for fast org lookups
CREATE INDEX IF NOT EXISTS idx_authorized_orgs_org
  ON authorized_orgs(LOWER(github_org)) WHERE enabled = TRUE;

-- Trigger to auto-update updated_at
DROP TRIGGER IF EXISTS update_authorized_orgs_updated_at ON authorized_orgs;
CREATE TRIGGER update_authorized_orgs_updated_at
  BEFORE UPDATE ON authorized_orgs
  FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
