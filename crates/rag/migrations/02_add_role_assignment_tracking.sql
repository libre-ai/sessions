-- I2: Add role_assignment tracking to sessions.
-- Stores RoleAssignment JSON array per session (for audit/recovery).

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS role_assignments JSONB DEFAULT '[]';
-- role_assignments: JSON array of RoleAssignment objects (actor_id, role, permissions)
