# Distributed Block Builder

This is a complete, compilable implementation of the distributed block building protocol that enables BuilderNet to scale to thousands of nodes while maintaining low latency and high performance.

## Architecture Overview

The implementation follows a **heterogeneous two-tier architecture** designed to solve the quadratic scaling problems of all-to-all orderflow sharing:

### Key Design Principles

1. **Role Separation**: Nodes are divided into two distinct roles:
   - **Simulation Nodes (~90%)**: Handle transaction simulation and partial block construction
   - **Merging Nodes (~10%)**: Merge partial blocks and submit to relays

2. **No Orderflow Gossip**: Unlike traditional gossip networks, **orderflow is NOT gossiped between simulation nodes**. Instead:
   - Simulation nodes send partial blocks directly to merging nodes
   - Gossip is used ONLY for network topology discovery and bundle replacements

3. **Geographic Awareness**: 
   - Minimizes cross-region communication
   - Ensures at most one hop from transaction receipt to relay submission
   - Simulation nodes prefer same-region mergers

4. **Deduplication**:
   - Each transaction is simulated as few times as possible without negatively impacting performance
   - Only merging nodes save redistribution archive data
   - Only merging nodes submit blocks to relays

## Key Features Implemented

### 1. **Partial Block Construction**
- Simulation nodes build conflict-free sets of transactions
- Include pre-computed simulation results

### 2. **Conflict Detection**
- Tracks touched accounts, storage slots, and consumed nonces
- Enables parallel processing without conflicts
- Cherry-picking of non-conflicting transactions

### 3. **Direct Routing**
- Simulation nodes send partial blocks directly to mergers
- Latency-sensitive orderflow goes straight to mergers
- No peer-to-peer orderflow sharing between simulation nodes

### 4. **Network Topology**
- Self-organizing based on relay latencies
- Geographic clustering to minimize latency
- Dynamic role assignment through consensus

## Implementation Details

### Transaction Flow

1. **Regular Transactions**:
   - Submitted to any simulation node
   - Simulated once and included in partial block
   - Partial block sent directly to nearby mergers
   - NO gossiping to other simulation nodes

2. **Latency-Sensitive Transactions**:
   - Submitted directly to merging nodes
   - Processed with priority in final block assembly
   - Saves one network hop for critical orderflow

3. **Replacements**:
   - Can be submitted to any node
   - Gossiped through network for quick propagation
   - Mergers coordinate to ensure consistency

### Role Assignment

Nodes self-organize based on:
- Relay latencies (nodes with <10ms latency become mergers)
- Geographic distribution (ensures mergers in each region)
- Network capacity (maintains ~10% merger ratio)

### Data Management

- **Simulation Nodes**: 
  - Store transaction data
  - Do NOT store built block data
  - Send partial blocks as conflict-free sets

- **Merging Nodes**:
  - Store redistribution archive data
  - Save only winning block data (future optimization)
  - Coordinate with other mergers for redundancy

## Building and Running

### Prerequisites
- Rust 1.70 or later
- Cargo

### Build the project
```bash
cargo build --release
```

### Run network example
```bash
cargo run --example start_network
```

### Run tests
```bash
# Full protocol test
cargo test test_full_protocol_flow -- --nocapture

# Specific tests
cargo test test_conflict_detection
cargo test test_geographic_routing
cargo test test_role_assignment
```

## Configuration

Key parameters:
- `MERGERS_FRACTION`: 0.1 (10% of nodes become mergers)
- `PARTIAL_BLOCK_INTERVAL`: 100ms
- `MAX_MERGER_CONNECTIONS`: 5 (each simulator connects to max 5 mergers)

## Network Behavior

### At 100 nodes:
- ~10 merging nodes, ~90 simulation nodes
- Each simulation node sends to ~5 nearby mergers
- Total messages: O(450) per partial block interval
- Relay submissions: 10 nodes maximum

### At 1000 nodes:
- ~100 merging nodes, ~900 simulation nodes  
- Each simulation node sends to ~5 nearby mergers
- Total messages: O(4500) per partial block interval
- Relay submissions: 100 nodes (manageable with relay coordination)

### At 10,000 nodes:
- Would require additional aggregation layer
- 3-tier architecture: simulation → aggregation → merging
- Still maintains O(N) communication complexity

## Future Enhancements

1. **Dynamic Topology Management**: Implement consensus-based role assignment
2. **Archival Optimization**: Save only winning blocks with diff-based storage
3. **Multi-Region Coordination**: Optimize cross-region merger communication
4. **Cancellation Optimization**: Targeted cancellation routing to affected nodes
5. **Reputation System**: Track which users submit to multiple nodes inappropriately
