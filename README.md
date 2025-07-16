# Distributed Block Builder

This is a complete, compilable implementation of the distributed block building protocol described in the design document. The implementation demonstrates parallel block construction using a two-tier gossip network with geographic awareness and role specialization.

## Key Features Implemented

### 1. **Two-Tier Architecture**
- **Simulation Nodes (80%)**: Handle transaction simulation and partial block construction
- **Merging Nodes (20%)**: Merge partial blocks and handle latency-sensitive orderflow

### 2. **Conflict Detection**
- Tracks touched accounts, storage slots, and consumed nonces
- Enables safe parallel processing without re-simulation
- Cherry-picking of non-conflicting transactions from multiple partial blocks

### 3. **Geographic-Aware Gossip**
- Prefers same-region peers to reduce cross-region traffic
- Implements selective gossip with configurable fanout
- Priority propagation for cancellations

### 4. **Orderflow Types**
- Regular orderflow: Distributed via gossip network
- Latency-sensitive: Sent directly to merging nodes

### 5. **Partial Block Construction**
- Simulation nodes build partial blocks every 100ms
- Include pre-computed simulation results
- Merging nodes combine partial blocks intelligently

## Project Structure

```
distributed-block-builder/
├── Cargo.toml              # Project dependencies
├── src/
│   └── main.rs            # Main implementation
└── tests/
    └── integration_test.rs # Comprehensive tests
```

## Building and Running

### Prerequisites
- Rust 1.70 or later
- Cargo

### Build the project
```bash
cargo build --release
```

### Run the main binary
```bash
cargo run
```

### Run the network example
```bash
cargo run --example start_network
```

### Run tests
```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_full_protocol_flow -- --nocapture

# Using the helper script
./run.sh test-protocol
```

## Implementation Details

### Network Transport
The implementation includes a simulated network transport layer that:
- Routes messages between nodes
- Simulates network latency based on geographic regions
- Handles async message passing via channels

### Simulation Engine
A simplified simulation engine that:
- Tracks account touches and nonce consumption
- Calculates gas usage and profit estimates
- Validates transaction/bundle execution

### Conflict Resolution
The merging algorithm:
1. Sorts partial blocks by profit per gas
2. Iterates through items, checking conflicts
3. Includes non-conflicting items
4. Adds latency-sensitive items last
Note: an actual implementation would have to include latency-sensitive OF in parallel (treating it as a megabundle), or at the top of the block.

### Role Assignment
Nodes self-organize based on:
- Relay latencies (lower latency nodes become mergers)
- Geographic distribution
- Network capacity

## Test Suite

### 1. **End-to-End Protocol Test** (`test_full_protocol_flow`)
- Starts 10 nodes across 3 regions
- Generates mixed orderflow for 5 seconds
- Verifies partial block creation and merging

### 2. **Conflict Detection Test** (`test_conflict_detection`)
- Tests account conflict detection
- Tests nonce conflict detection
- Verifies non-conflicting sets work correctly

### 3. **Geographic Routing Test** (`test_geographic_routing`)
- Verifies same-region peer preference
- Tests latency-based routing decisions

## Key Optimizations Demonstrated

1. **Simulation Deduplication**: Each transaction simulated only once
2. **Parallel Processing**: Multiple nodes build partial blocks simultaneously
3. **Efficient Merging**: No re-simulation needed during merge
4. **Geographic Awareness**: Reduces cross-region bandwidth usage

Notes:
* Importantly, all orderflow will reach a merger in one hop with high probability
* Users can still submit directly to many nodes to save on some latency without negatively impacting performance of the system

## Scaling Properties

- **Communication**: O(n) via gossip instead of O(n²). This lets us scale to many more nodes (1000+)
- **Simulation**: O(t) total simulations instead of O(n×t). This lets us scale orderflow proportionally to # of nodes!
- **Latency**: Maintained through role separation. With high probability orderflow reaches a relay with only a single additional (short) hop.
- **Reliability**: Natural redundancy through gossip.
- **Archive data**: O(mergers) instead of O(nodes) writers to redistribution archive. This reduces the cost of 
    This is achieved through deduplication at the mergers rather than 
    Note: mergers should send their payloads to other mergers for much improved availability of data!
    Note: with enough replication we could only ever save a single winning block rather than all blocks built.
    Note: for the current redistribution, transactions considered have to be managed per partial as well as merged block! But merger nodes can keep track of this data somewhat easily.
    Note: transactions considered is append-only, and now only kept per-merger.
    Note: to further reduce performance requirements we can keep diffs of blocks built per merger.
    With all the above improvements, redistribution archive can possibly be realized without a centralized database.

## Future Enhancements

1. **Persistent Storage**: Add database for orderflow history
2. **Real Network Transport**: Replace simulated transport with actual P2P
3. **Advanced Heuristics**: Implement sophisticated transaction ordering
4. **MEV Strategies**: Add MEV extraction algorithms
5. **Monitoring**: Add Prometheus metrics and dashboards

## Running a Test Scenario

To see the protocol in action with detailed output:

```bash
cargo test test_full_protocol_flow -- --nocapture
```

This will:
1. Start 10 nodes across 3 geographic regions
2. Generate transactions and bundles for 5 seconds
3. Show partial block creation and merging
4. Display final metrics

## Example Output

Running the full protocol test:
```
=== Distributed Block Building Protocol Test ===

Started node 0001 in region us-east as Merging
Started node 0102 in region us-east as Simulation
Started node 0203 in region us-east as Simulation
Started node 0304 in region us-east as Simulation
Started node 0405 in region eu-west as Merging
Started node 0506 in region eu-west as Simulation
Started node 0607 in region eu-west as Simulation
Started node 0708 in region asia-pac as Simulation
Started node 0809 in region asia-pac as Simulation
Started node 090a in region asia-pac as Simulation

=== Generating orderflow for 5 seconds ===

Sending transaction a3b2 to node 0102
Sending bundle 5f3a to node 0203
Sending transaction 7c4d to node 0506
Sending latency-sensitive tx to merging node 0001
Node 0102 sending partial block to merger 0001
Node 0203 sending partial block to merger 0001
Merging node 0001 built block 1 with 42 items
Sending bundle 8e5f to node 0708
Node 0506 sending partial block to merger 0405
Merging node 0405 built block 1 with 38 items

=== Test Metrics ===
Transactions sent: 450
Bundles sent: 90
Partial blocks created: 45
Final blocks built: 5
Conflicts detected: 12
Total messages routed: 1823

=== Test completed successfully ===
```

Running the network example:
```
Starting Distributed Block Building Network
==========================================

Starting 4 nodes in us-east
Node 3f2a started in us-east
Node 5c8d started in us-east
Node 1a9e started in us-east
Node 7b4f started in us-east
Starting 3 nodes in eu-west
Node 9d61 started in eu-west
Node 2e73 started in eu-west
Node 6f84 started in eu-west
Starting 3 nodes in asia-pac
Node 8a95 started in asia-pac
Node 4ba6 started in asia-pac
Node 0cb7 started in asia-pac

All nodes started. Network is initializing...

Generating orderflow...
TX 1 → Node 5c8d
TX 2 → Node 1a9e
Bundle 1 → Node 9d61
TX 3 → Node 7b4f
Latency-sensitive TX 4 sent
TX 5 → Node 2e73
Running for 30 seconds...

Shutting down network...
Stopping node 3f2a in us-east
Stopping node 5c8d in us-east
...

Network shutdown complete.
```
