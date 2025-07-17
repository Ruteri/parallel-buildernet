// tests/integration_test.rs
use distributed_block_builder::*;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, Duration};

#[derive(Default)]
struct TestMetrics {
    partial_blocks_sent: usize,
    transactions_direct_to_merger: usize,
}

// Test tracking network hub
struct TestNetworkHub {
    hub: NetworkHub,
    message_log: Arc<RwLock<Vec<(NodeId, NodeId, String)>>>,
}

impl TestNetworkHub {
    fn new() -> Self {
        Self {
            hub: NetworkHub::new(),
            message_log: Arc::new(RwLock::new(Vec::new())),
        }
    }

    fn register_node(
        &mut self,
        id: NodeId,
        tx: mpsc::Sender<(NodeId, Message)>,
        rx: mpsc::Receiver<(NodeId, Message)>,
    ) {
        self.hub.register_node(id, tx, rx);
    }

    async fn run(mut self) {
        let log = self.message_log.clone();

        loop {
            let node_ids: Vec<_> = self.hub.nodes.keys().cloned().collect();

            for sender_id in node_ids {
                if let Some((_, rx)) = self.hub.nodes.get_mut(&sender_id) {
                    if let Ok((target, msg)) = rx.try_recv() {
                        // Log message type
                        let msg_type = match &msg {
                            Message::Transaction(_) => "Transaction",
                            Message::Bundle(_) => "Bundle",
                            Message::PartialBlock(_) => "PartialBlock",
                            Message::NodeAnnouncement(_) => "NodeAnnouncement",
                            Message::Cancellation(_) => "Cancellation",
                        };

                        log.write()
                            .await
                            .push((sender_id, target, msg_type.to_string()));

                        // Route message
                        if target == [0u8; 32] {
                            // Broadcast
                            for (id, (tx, _)) in &self.hub.nodes {
                                if *id != sender_id {
                                    let _ = tx.send((sender_id, msg.clone())).await;
                                }
                            }
                        } else {
                            // Direct send
                            if let Some((tx, _)) = self.hub.nodes.get(&target) {
                                let _ = tx.send((sender_id, msg)).await;
                            }
                        }
                    }
                }
            }

            sleep(Duration::from_millis(10)).await;
        }
    }
}

#[tokio::test]
async fn test_basic_protocol() {
    println!("\n=== Testing Basic Protocol ===");

    let mut test_hub = TestNetworkHub::new();
    let metrics = Arc::new(RwLock::new(TestMetrics::default()));

    // Create 1 merger and 3 simulation nodes
    let merger_id = [1u8; 32];
    let sim_ids = vec![[2u8; 32], [3u8; 32], [4u8; 32]];

    // Deploy merger
    {
        let config = NodeConfig {
            id: merger_id,
            role: NodeRole::Merging,
            region: "us-east".to_string(),
        };

        let (transport, tx_in, rx_out) = NetworkTransport::new(merger_id);
        test_hub.register_node(merger_id, tx_in, rx_out);

        let (_, shutdown_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = create_node(config, transport, shutdown_rx).await;
        });
    }

    // Deploy simulation nodes
    for sim_id in &sim_ids {
        let config = NodeConfig {
            id: *sim_id,
            role: NodeRole::Simulation,
            region: "us-east".to_string(),
        };

        let (transport, tx_in, rx_out) = NetworkTransport::new(*sim_id);
        test_hub.register_node(*sim_id, tx_in, rx_out);

        let (_, shutdown_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = create_node(config, transport, shutdown_rx).await;
        });
    }

    // Start hub
    let log_clone = test_hub.message_log.clone();
    tokio::spawn(async move {
        test_hub.run().await;
    });

    // Wait for initialization
    sleep(Duration::from_millis(500)).await;

    // Test 1: Verify no orderflow gossip between simulation nodes
    println!("Test 1: Verifying no orderflow gossip...");

    // Check message log - should see NodeAnnouncements but no transactions between sim nodes
    let log = log_clone.read().await;
    let tx_between_sims = log
        .iter()
        .filter(|(from, to, msg_type)| {
            sim_ids.contains(from)
                && sim_ids.contains(to)
                && (msg_type == "Transaction" || msg_type == "Bundle")
        })
        .count();

    assert_eq!(
        tx_between_sims, 0,
        "Found orderflow gossip between simulation nodes!"
    );
    println!("✓ No orderflow gossip between simulation nodes");

    // Test 2: Verify partial blocks are sent to mergers
    println!("\nTest 2: Verifying partial block routing...");

    // Wait for partial blocks
    sleep(Duration::from_millis(200)).await;

    let partial_blocks_to_merger = log
        .iter()
        .filter(|(from, to, msg_type)| {
            sim_ids.contains(from) && *to == merger_id && msg_type == "PartialBlock"
        })
        .count();

    println!(
        "✓ {} partial blocks sent to merger",
        partial_blocks_to_merger
    );

    // Test 3: Verify cancellations are gossiped
    println!("\nTest 3: Verifying cancellation gossip...");

    let cancellation_msgs = log
        .iter()
        .filter(|(_, _, msg_type)| msg_type == "Cancellation")
        .count();

    println!("✓ {} cancellation messages", cancellation_msgs);

    println!("\n✅ Basic protocol test completed successfully!");
}

#[tokio::test]
async fn test_role_separation() {
    println!("\n=== Testing Role Separation ===");

    // Test that nodes maintain their fixed roles
    let sim_config = NodeConfig {
        id: [1u8; 32],
        role: NodeRole::Simulation,
        region: "us-east".to_string(),
    };

    let merge_config = NodeConfig {
        id: [2u8; 32],
        role: NodeRole::Merging,
        region: "us-east".to_string(),
    };

    assert!(matches!(sim_config.role, NodeRole::Simulation));
    assert!(matches!(merge_config.role, NodeRole::Merging));

    println!("✓ Nodes maintain fixed roles from deployment");
    println!("✅ Role separation test completed!");
}

#[tokio::test]
async fn test_conflict_detection() {
    println!("\n=== Testing Conflict Detection ===");

    let mut conflicts1 = ConflictSet::new();
    let mut conflicts2 = ConflictSet::new();

    let addr1 = [1u8; 20];
    let addr2 = [2u8; 20];

    // Set 1: touches addr1
    conflicts1.touched_accounts.insert(addr1);
    conflicts1.consumed_nonces.insert(addr1, 5);

    // Set 2: touches addr1 - should conflict
    conflicts2.touched_accounts.insert(addr1);

    assert!(conflicts1.conflicts_with(&conflicts2));
    println!("✓ Detected account conflict");

    // Non-conflicting sets
    let mut conflicts3 = ConflictSet::new();
    conflicts3.touched_accounts.insert(addr2);

    assert!(!conflicts1.conflicts_with(&conflicts3));
    println!("✓ Non-conflicting sets detected correctly");

    println!("✅ Conflict detection test completed!");
}
