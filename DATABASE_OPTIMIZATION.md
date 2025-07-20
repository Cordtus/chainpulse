# ChainPulse Database Optimization Guide

## Current Performance Analysis

Based on testing with real data, the database structure is performing well. Here's the analysis:

### Index Usage
- All indexes are being utilized effectively
- The partial indexes (WHERE clauses) are reducing index size significantly
- Most selective indexes: `packets_data_hash` (167 rows, 2 unique values per entry)

## Optimization Recommendations

### 1. Storage Optimizations

#### Use INTEGER for Timestamps
Currently using INTEGER for timeout_timestamp (nanoseconds). Consider:
- Store as milliseconds instead of nanoseconds to save space
- Or use SQLite's built-in datetime functions with indexed computed columns

#### Compress Data Hash
- Current: 64-character hex string (64 bytes)
- Optimized: Store as BLOB (32 bytes) - 50% space saving

```sql
-- Migration to optimize storage
ALTER TABLE packets ADD COLUMN data_hash_blob BLOB;
UPDATE packets SET data_hash_blob = unhex(data_hash);
-- Then drop old column and rename
```

### 2. Query Performance Optimizations

#### Add Composite Indexes for Common Queries
```sql
-- For user transfer queries
CREATE INDEX packets_user_transfers ON packets(sender, receiver, effected, created_at);

-- For channel performance analytics
CREATE INDEX packets_channel_analytics ON packets(src_channel, dst_channel, effected, created_at);

-- For timeout monitoring
CREATE INDEX packets_timeout_monitor ON packets(timeout_timestamp, effected, src_channel, dst_channel) 
    WHERE timeout_timestamp IS NOT NULL AND effected = 0;
```

#### Materialized Views for Analytics
For frequently accessed analytics, consider materialized views:

```sql
-- Channel statistics view
CREATE VIEW channel_stats AS
SELECT 
    src_channel,
    dst_channel,
    COUNT(*) as total_packets,
    SUM(CASE WHEN effected = 1 THEN 1 ELSE 0 END) as delivered,
    AVG(CASE WHEN effected = 1 
        THEN julianday(effected_at) - julianday(created_at) 
        ELSE NULL END) * 86400 as avg_delivery_seconds,
    SUM(CAST(amount AS INTEGER)) as total_volume
FROM packets
WHERE created_at > datetime('now', '-7 days')
GROUP BY src_channel, dst_channel;

CREATE INDEX channel_stats_idx ON channel_stats(src_channel, dst_channel);
```

### 3. Database Configuration

#### Enable Write-Ahead Logging (WAL)
Already enabled, good for concurrent reads

#### Optimize SQLite Settings
```sql
PRAGMA page_size = 4096;  -- Optimal for most workloads
PRAGMA cache_size = -64000;  -- 64MB cache
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 30000000000;  -- 30GB memory-mapped I/O
PRAGMA synchronous = NORMAL;  -- Balance between safety and speed
```

### 4. Partitioning Strategy

For long-term scalability, consider:

#### Time-based Partitioning
- Create monthly tables: `packets_2024_01`, `packets_2024_02`, etc.
- Use a view to union current and recent months
- Archive old months to separate databases

#### Channel-based Sharding
- For very high volume, shard by channel hash
- Separate databases for major channels

### 5. Data Lifecycle Management

#### Implement Data Retention
```sql
-- Archive old completed packets
CREATE TABLE packets_archive AS 
SELECT * FROM packets 
WHERE effected = 1 
  AND created_at < datetime('now', '-30 days');

DELETE FROM packets 
WHERE effected = 1 
  AND created_at < datetime('now', '-30 days');
```

#### Vacuum Regularly
```bash
# Add to cron
0 3 * * * sqlite3 /path/to/chainpulse.db "VACUUM;"
```

### 6. Alternative Storage Engines

For extreme scale, consider:

1. **PostgreSQL with TimescaleDB**
   - Better for time-series data
   - Native partitioning
   - Better concurrent write performance

2. **ClickHouse**
   - Excellent for analytics queries
   - Columnar storage
   - Built for time-series data

3. **DuckDB**
   - Embedded like SQLite but optimized for analytics
   - Columnar storage
   - Better for complex queries

## Implementation Priority

1. **Immediate** (No downtime):
   - Add composite indexes
   - Optimize SQLite settings
   - Set up regular VACUUM

2. **Short-term** (Minutes of downtime):
   - Convert data_hash to BLOB
   - Add materialized views

3. **Long-term** (Planned migration):
   - Implement partitioning
   - Consider alternative storage engines if volume exceeds 100GB

## Monitoring

Add these queries to monitor database health:

```sql
-- Table size
SELECT 
    name,
    SUM(pgsize) as size_bytes,
    SUM(pgsize)/1024/1024 as size_mb
FROM dbstat
WHERE name LIKE 'packets%'
GROUP BY name;

-- Index effectiveness
SELECT 
    name,
    idx,
    stat
FROM sqlite_stat1
WHERE tbl = 'packets'
ORDER BY CAST(substr(stat, 1, instr(stat, ' ') - 1) AS INTEGER) DESC;
```