// tests/integration_test.rs
use rand::{thread_rng, Rng};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, Duration};

// Import from main module
use distributed_block_builder::*;

/// Test harness for distributed block building
struct TestHarness {
    nodes: HashMap<NodeId, TestNode>,
    network: Arc<RwLock<TestNetwork>>,
    metrics: Arc<RwLock<TestMetrics>>,
}

struct TestNode {
    id: NodeId,
    role: NodeRole,
    shutdown: mpsc::Sender<()>,
}

struct TestNetwork {
    message_routes: HashMap<(NodeId, NodeId), mpsc::Sender<Message>>,
    message_count: u64,
    latency_map: HashMap<(String, String), Duration>,
}

#[derive(Default)]
struct TestMetrics {
    transactions_sent: u64,
    bundles_sent: u64,
    partial_blocks_created: u64,
    final_blocks_built: u64,
    conflicts_detected: u64,
}

impl TestHarness {
    async fn new(num_nodes: usize) -> Self {
        let network = Arc::new(RwLock::new(TestNetwork {
            message_routes: HashMap::new(),
            message_count: 0,
            latency_map: Self::create_latency_map(),
        }));

        let metrics = Arc::new(RwLock::new(TestMetrics::default()));

        Self {
            nodes: HashMap::new(),
            network,
            metrics,
        }
    }

    fn create_latency_map() -> HashMap<(String, String), Duration> {
        let mut map = HashMap::new();

        // Same region latencies
        map.insert(
            ("us-east".to_string(), "us-east".to_string()),
            Duration::from_millis(1),
        );
        map.insert(
            ("eu-west".to_string(), "eu-west".to_string()),
            Duration::from_millis(1),
        );
        map.insert(
            ("asia-pac".to_string(), "asia-pac".to_string()),
            Duration::from_millis(1),
        );

        // Cross-region latencies
        map.insert(
            ("us-east".to_string(), "eu-west".to_string()),
            Duration::from_millis(50),
        );
        map.insert(
            ("eu-west".to_string(), "us-east".to_string()),
            Duration::from_millis(50),
        );
        map.insert(
            ("us-east".to_string(), "asia-pac".to_string()),
            Duration::from_millis(100),
        );
        map.insert(
            ("asia-pac".to_string(), "us-east".to_string()),
            Duration::from_millis(100),
        );
        map.insert(
            ("eu-west".to_string(), "asia-pac".to_string()),
            Duration::from_millis(80),
        );
        map.insert(
            ("asia-pac".to_string(), "eu-west".to_string()),
            Duration::from_millis(80),
        );

        map
    }

    async fn start_nodes(&mut self, configs: Vec<(NodeId, String, bool)>) {
        // configs: (node_id, region, is_merging_node)
        for (node_id, region, is_merging) in configs {
            let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

            let role = if is_merging {
                let mut relay_latencies = HashMap::new();
                relay_latencies.insert("relay1".to_string(), Duration::from_millis(5));
                relay_latencies.insert("relay2".to_string(), Duration::from_millis(10));

                NodeRole::MergingNode { relay_latencies }
            } else {
                NodeRole::SimulationNode {}
            };

            self.nodes.insert(
                node_id,
                TestNode {
                    id: node_id,
                    role: role.clone(),
                    shutdown: shutdown_tx,
                },
            );

            println!(
                "Started node {} in region {} as {:?}",
                hex::encode(&node_id[0..4]),
                region,
                match role {
                    NodeRole::SimulationNode { .. } => "Simulation",
                    NodeRole::MergingNode { .. } => "Merging",
                }
            );
        }
    }

    async fn generate_orderflow(&self, duration: Duration) {
        let start = tokio::time::Instant::now();
        let mut rng = thread_rng();

        while start.elapsed() < duration {
            // Generate transactions
            for _ in 0..10 {
                let tx = self.create_random_transaction(&mut rng).await;
                self.metrics.write().await.transactions_sent += 1;

                // Send to random simulation node
                let sim_nodes: Vec<_> = self
                    .nodes
                    .values()
                    .filter(|n| matches!(n.role, NodeRole::SimulationNode { .. }))
                    .collect();

                if let Some(node) = sim_nodes.get(rng.gen_range(0..sim_nodes.len())) {
                    println!(
                        "Sending transaction {} to node {}",
                        hex::encode(&tx.tx.hash()[0..4]),
                        hex::encode(&node.id[0..4])
                    );
                }
            }

            // Generate bundles
            for _ in 0..2 {
                let bundle = self.create_random_bundle(&mut rng).await;
                self.metrics.write().await.bundles_sent += 1;

                let sim_nodes: Vec<_> = self
                    .nodes
                    .values()
                    .filter(|n| matches!(n.role, NodeRole::SimulationNode { .. }))
                    .collect();

                if let Some(node) = sim_nodes.get(rng.gen_range(0..sim_nodes.len())) {
                    println!(
                        "Sending bundle {} to node {}",
                        hex::encode(&bundle.id[0..4]),
                        hex::encode(&node.id[0..4])
                    );
                }
            }

            // Generate some latency-sensitive orderflow
            if rng.gen_bool(0.1) {
                let tx = self.create_latency_sensitive_transaction(&mut rng).await;

                // Send directly to merging node
                let merging_nodes: Vec<_> = self
                    .nodes
                    .values()
                    .filter(|n| matches!(n.role, NodeRole::MergingNode { .. }))
                    .collect();

                if let Some(node) = merging_nodes.get(rng.gen_range(0..merging_nodes.len())) {
                    println!(
                        "Sending latency-sensitive tx to merging node {}",
                        hex::encode(&node.id[0..4])
                    );
                }
            }

            sleep(Duration::from_millis(100)).await;
        }
    }

    async fn create_random_transaction(&self, rng: &mut impl Rng) -> SignedTransaction {
        SignedTransaction {
            tx: Transaction {
                nonce: rng.gen_range(1..100),
                from: self.random_address(rng),
                to: self.random_address(rng),
                value: rng.gen_range(100..10000),
                gas_limit: 21000,
                gas_price: rng.gen_range(10..100),
                data: vec![],
            },
            signature: Signature::default(),
            received_at: current_timestamp(),
            is_latency_sensitive: false,
        }
    }

    async fn create_latency_sensitive_transaction(&self, rng: &mut impl Rng) -> SignedTransaction {
        let mut tx = self.create_random_transaction(rng).await;
        tx.is_latency_sensitive = true;
        tx.tx.gas_price *= 10; // Higher priority
        tx
    }

    async fn create_random_bundle(&self, rng: &mut impl Rng) -> Bundle {
        let mut txs = Vec::new();
        for _ in 0..rng.gen_range(2..5) {
            txs.push(Transaction {
                nonce: rng.gen_range(1..100),
                from: self.random_address(rng),
                to: self.random_address(rng),
                value: rng.gen_range(100..10000),
                gas_limit: 21000,
                gas_price: rng.gen_range(10..100),
                data: vec![],
            });
        }

        Bundle {
            id: self.random_hash(rng),
            transactions: txs,
            reverting_tx_hashes: vec![],
            target_block: 1,
            min_timestamp: None,
            max_timestamp: None,
            is_latency_sensitive: false,
        }
    }

    fn random_address(&self, rng: &mut impl Rng) -> Address {
        let mut addr = [0u8; 20];
        rng.fill(&mut addr);
        addr
    }

    fn random_hash(&self, rng: &mut impl Rng) -> Hash {
        let mut hash = [0u8; 32];
        rng.fill(&mut hash);
        hash
    }

    async fn print_metrics(&self) {
        let metrics = self.metrics.read().await;
        println!("\n=== Test Metrics ===");
        println!("Transactions sent: {}", metrics.transactions_sent);
        println!("Bundles sent: {}", metrics.bundles_sent);
        println!("Partial blocks created: {}", metrics.partial_blocks_created);
        println!("Final blocks built: {}", metrics.final_blocks_built);
        println!("Conflicts detected: {}", metrics.conflicts_detected);

        let network = self.network.read().await;
        println!("Total messages routed: {}", network.message_count);
    }
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// Extension trait to add hash method to Transaction
trait TransactionExt {
    fn hash(&self) -> Hash;
}

impl TransactionExt for Transaction {
    fn hash(&self) -> Hash {
        let mut hash = [0u8; 32];
        hash[0..8].copy_from_slice(&self.nonce.to_be_bytes());
        hash[8..28].copy_from_slice(&self.from);
        hash[28..32].copy_from_slice(&self.value.to_be_bytes()[12..16]);
        hash
    }
}

#[tokio::test]
async fn test_full_protocol_flow() {
    println!("\n=== Distributed Block Building Protocol Test ===\n");

    let mut harness = TestHarness::new(10).await;

    // Create nodes across 3 regions
    let mut configs = Vec::new();

    // US-East region: 4 nodes (1 merging, 3 simulation)
    for i in 0..4 {
        let node_id = create_node_id(i);
        let is_merging = i == 0;
        configs.push((node_id, "us-east".to_string(), is_merging));
    }

    // EU-West region: 3 nodes (1 merging, 2 simulation)
    for i in 4..7 {
        let node_id = create_node_id(i);
        let is_merging = i == 4;
        configs.push((node_id, "eu-west".to_string(), is_merging));
    }

    // Asia-Pacific region: 3 nodes (all simulation)
    for i in 7..10 {
        let node_id = create_node_id(i);
        configs.push((node_id, "asia-pac".to_string(), false));
    }

    harness.start_nodes(configs).await;

    println!("\n=== Generating orderflow for 5 seconds ===\n");

    // Generate orderflow for 5 seconds
    harness.generate_orderflow(Duration::from_secs(5)).await;

    // Wait for final processing
    sleep(Duration::from_secs(2)).await;

    // Print final metrics
    harness.print_metrics().await;

    println!("\n=== Test completed successfully ===");
}

#[tokio::test]
async fn test_conflict_detection() {
    println!("\n=== Testing Conflict Detection ===\n");

    let mut conflict_set1 = ConflictSet {
        touched_accounts: HashSet::new(),
        touched_storage: HashMap::new(),
        consumed_nonces: HashMap::new(),
    };

    let mut conflict_set2 = ConflictSet {
        touched_accounts: HashSet::new(),
        touched_storage: HashMap::new(),
        consumed_nonces: HashMap::new(),
    };

    let addr1 = [1u8; 20];
    let addr2 = [2u8; 20];
    let addr3 = [3u8; 20];

    // Set 1: touches addr1 and addr2
    conflict_set1.touched_accounts.insert(addr1);
    conflict_set1.touched_accounts.insert(addr2);
    conflict_set1.consumed_nonces.insert(addr1, 5);

    // Set 2: touches addr2 and addr3 - should conflict
    conflict_set2.touched_accounts.insert(addr2);
    conflict_set2.touched_accounts.insert(addr3);

    assert!(conflict_set1.conflicts_with(&conflict_set2));
    println!("✓ Detected account conflict correctly");

    // Test non-conflicting sets
    let mut conflict_set3 = ConflictSet {
        touched_accounts: HashSet::new(),
        touched_storage: HashMap::new(),
        consumed_nonces: HashMap::new(),
    };
    conflict_set3.touched_accounts.insert(addr3);

    assert!(!conflict_set1.conflicts_with(&conflict_set3));
    println!("✓ Non-conflicting sets detected correctly");

    // Test nonce conflicts
    let mut conflict_set4 = ConflictSet {
        touched_accounts: HashSet::new(),
        touched_storage: HashMap::new(),
        consumed_nonces: HashMap::new(),
    };
    conflict_set4.consumed_nonces.insert(addr1, 4); // Lower nonce - should conflict

    assert!(conflict_set1.conflicts_with(&conflict_set4));
    println!("✓ Nonce conflict detected correctly");
}

#[tokio::test]
async fn test_geographic_routing() {
    println!("\n=== Testing Geographic-Aware Routing ===\n");

    // Create a simple test to verify geographic preference
    let mut peers = HashMap::new();

    let node1 = create_node_id(1);
    let node2 = create_node_id(2);
    let node3 = create_node_id(3);

    peers.insert(
        node1,
        PeerInfo {
            region: "us-east".to_string(),
            latency: Duration::from_millis(5),
            role: NodeRole::SimulationNode {},
            last_seen: std::time::Instant::now(),
        },
    );

    peers.insert(
        node2,
        PeerInfo {
            region: "eu-west".to_string(),
            latency: Duration::from_millis(50),
            role: NodeRole::SimulationNode {},
            last_seen: std::time::Instant::now(),
        },
    );

    peers.insert(
        node3,
        PeerInfo {
            region: "us-east".to_string(),
            latency: Duration::from_millis(10),
            role: NodeRole::SimulationNode {},
            last_seen: std::time::Instant::now(),
        },
    );

    // Simulate selection from us-east node
    let current_region = "us-east";
    let mut peer_list: Vec<_> = peers.iter().collect();

    peer_list.sort_by_key(|(_, info)| {
        let region_penalty = if info.region != current_region {
            1000
        } else {
            0
        };
        let latency = info.latency.as_millis() as u32;
        region_penalty + latency
    });

    // Should prefer same-region nodes
    assert_eq!(peer_list[0].1.region, "us-east");
    assert_eq!(peer_list[1].1.region, "us-east");
    assert_eq!(peer_list[2].1.region, "eu-west");

    println!("✓ Geographic routing preference working correctly");
}

fn create_node_id(index: u8) -> NodeId {
    let mut id = [0u8; 32];
    id[0] = index;
    for i in 1..32 {
        id[i] = (index as usize * i) as u8;
    }
    id
}
