-- Add user data columns to packets table for packet clearing feature
-- These columns are nullable to maintain backward compatibility

-- Add columns for fungible token packet data
ALTER TABLE packets ADD COLUMN sender TEXT;
ALTER TABLE packets ADD COLUMN receiver TEXT;
ALTER TABLE packets ADD COLUMN denom TEXT;
ALTER TABLE packets ADD COLUMN amount TEXT;
ALTER TABLE packets ADD COLUMN ibc_version TEXT DEFAULT 'v1';

-- Create indexes for efficient user queries
-- Partial indexes only on non-null values to save space
CREATE INDEX idx_packets_sender ON packets(sender) WHERE sender IS NOT NULL;
CREATE INDEX idx_packets_receiver ON packets(receiver) WHERE receiver IS NOT NULL;
CREATE INDEX idx_packets_pending_by_sender ON packets(sender, effected) WHERE effected = 0 AND sender IS NOT NULL;
CREATE INDEX idx_packets_pending_by_receiver ON packets(receiver, effected) WHERE effected = 0 AND receiver IS NOT NULL;

-- Add composite index for channel-based stuck packet queries
CREATE INDEX idx_packets_stuck ON packets(src_channel, dst_channel, effected, created_at) WHERE effected = 0;