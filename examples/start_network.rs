use distributed_block_builder::*;
use rand::{thread_rng, Rng};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, sleep, Duration};

#[derive(Clone)]
struct NetworkStats {
    messages_sent: u64,
    partial_blocks_sent: u64,
    blocks_built: u64,
    transactions_simulated: u64,
}

struct LiveNetwork {
    nodes: HashMap<NodeId, NodeInfo>,
    stats: Arc<RwLock<NetworkStats>>,
}

struct NodeInfo {
    region: String,
    role: String,
    transport_tx: mpsc::Sender<(NodeId, Message)>,
}

impl LiveNetwork {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            stats: Arc::new(RwLock::new(NetworkStats {
                messages_sent: 0,
                partial_blocks_sent: 0,
                blocks_built: 0,
                transactions_simulated: 0,
            })),
        }
    }

    async fn register_node(
        &mut self,
        id: NodeId,
        region: String,
        role: String,
        tx: mpsc::Sender<(NodeId, Message)>,
    ) {
        self.nodes.insert(
            id,
            NodeInfo {
                region,
                role,
                transport_tx: tx,
            },
        );
    }

    async fn send_transaction(&self, node_id: NodeId, tx: SignedTransaction) {
        if let Some(node_info) = self.nodes.get(&node_id) {
            let _ = node_info
                .transport_tx
                .send((node_id, Message::Transaction(tx)))
                .await;

            let mut stats = self.stats.write().await;
            stats.messages_sent += 1;
            stats.transactions_simulated += 1;
        }
    }

    async fn send_bundle(&self, node_id: NodeId, bundle: Bundle) {
        if let Some(node_info) = self.nodes.get(&node_id) {
            let _ = node_info
                .transport_tx
                .send((node_id, Message::Bundle(bundle)))
                .await;

            let mut stats = self.stats.write().await;
            stats.messages_sent += 1;
        }
    }

    async fn get_simulation_nodes(&self) -> Vec<(NodeId, String)> {
        self.nodes
            .iter()
            .filter(|(_, info)| info.role == "Simulation")
            .map(|(id, info)| (*id, info.region.clone()))
            .collect()
    }

    async fn get_merging_nodes(&self) -> Vec<(NodeId, String)> {
        self.nodes
            .iter()
            .filter(|(_, info)| info.role == "Merging")
            .map(|(id, info)| (*id, info.region.clone()))
            .collect()
    }

    async fn monitor_network(&self) {
        let stats_clone = self.stats.clone();
        let mut monitor_interval = interval(Duration::from_secs(5));

        tokio::spawn(async move {
            loop {
                monitor_interval.tick().await;
                let stats = stats_clone.read().await;
                println!("\n📊 Network Statistics:");
                println!("   Messages sent: {}", stats.messages_sent);
                println!(
                    "   Transactions simulated: {}",
                    stats.transactions_simulated
                );
                println!("   Partial blocks sent: {}", stats.partial_blocks_sent);
                println!("   Blocks built: {}", stats.blocks_built);
            }
        });
    }
}

#[tokio::main]
async fn main() {
    println!("🚀 Starting Distributed Block Building Network");
    println!("============================================");
    println!("Architecture: Two-tier with direct routing");
    println!("Merger fraction: 10% (1 in 10 nodes)");
    println!("Communication: O(N) scaling\n");

    let regions = vec!["us-east", "eu-west", "asia-pac"];
    let nodes_per_region = vec![4, 3, 3];

    let mut network_hub = NetworkHub::new();
    let mut live_network = LiveNetwork::new();
    let mut nodes = Vec::new();
    let mut node_handles = Vec::new();

    // Start nodes in each region
    for (region_idx, (region, count)) in regions.iter().zip(nodes_per_region.iter()).enumerate() {
        println!("🌍 Starting {} nodes in {}", count, region);

        for i in 0..*count {
            let node_id = generate_node_id();
            let config = NodeConfig {
                id: node_id,
                region: region.to_string(),
                public_key: PublicKey::from([(region_idx * 10 + i) as u8; 33]),
                private_key: [(region_idx * 10 + i) as u8; 32],
            };

            let (transport, tx_in, rx_out) = NetworkTransport::new(node_id);
            network_hub.register_node(node_id, tx_in, rx_out);

            let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
            let node = Node::new(config.clone(), transport, shutdown_rx)
                .await
                .unwrap();

            nodes.push((node_id, region.to_string(), shutdown_tx));

            // Clone for network registration
            let tx_in_clone = tx_in.clone();

            let node_id_copy = node_id;
            let region_copy = region.to_string();
            let live_network_clone = live_network.stats.clone();

            node_handles.push(tokio::spawn(async move {
                // Monitor partial blocks and final blocks
                let stats = live_network_clone;

                match node.run().await {
                    Ok(_) => println!(
                        "✅ Node {} in {} shut down gracefully",
                        hex::encode(&node_id_copy[0..4]),
                        region_copy
                    ),
                    Err(e) => eprintln!(
                        "❌ Node {} in {} error: {}",
                        hex::encode(&node_id_copy[0..4]),
                        region_copy,
                        e
                    ),
                }
            }));

            // Wait a bit to see role assignment
            sleep(Duration::from_millis(150)).await;

            println!(
                "  ✨ Node {} started in {}",
                hex::encode(&node_id[0..4]),
                region
            );
        }
    }

    // Start network hub with statistics tracking
    let stats_for_hub = live_network.stats.clone();
    let hub_handle = tokio::spawn(async move {
        network_hub.run_with_stats(stats_for_hub).await;
    });

    println!("\n⏳ All nodes started. Network is initializing...");
    sleep(Duration::from_secs(3)).await;

    // After initialization, register nodes with live network
    println!("\n📋 Network Topology:");
    for (node_id, region, _) in &nodes {
        // Determine role (simplified - in reality this happens through consensus)
        let node_index = nodes.iter().position(|(id, _, _)| id == node_id).unwrap();
        let is_merger = node_index % 10 == 0; // Every 10th node is a merger

        let role = if is_merger { "Merging" } else { "Simulation" };

        println!(
            "   Node {} ({}) - Role: {}",
            hex::encode(&node_id[0..4]),
            region,
            role
        );

        // Note: In real implementation, we'd get the actual transport channel
        // For now, we'll use a dummy channel
        let (dummy_tx, _) = mpsc::channel(1);
        live_network
            .register_node(*node_id, region.clone(), role.to_string(), dummy_tx)
            .await;
    }

    // Start network monitoring
    live_network.monitor_network().await;

    // Generate orderflow
    println!("\n🔄 Generating orderflow...");
    let mut rng = thread_rng();
    let mut tx_count = 0;
    let mut bundle_count = 0;
    let mut latency_sensitive_count = 0;

    // Run for 30 seconds
    let start = tokio::time::Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        // Generate regular transactions (sent to simulation nodes)
        for _ in 0..5 {
            tx_count += 1;
            let tx = create_test_transaction(&mut rng, tx_count);

            // Pick a random simulation node
            let sim_nodes = live_network.get_simulation_nodes().await;
            if let Some((node_id, region)) = sim_nodes.get(rng.gen_range(0..sim_nodes.len().max(1)))
            {
                println!(
                    "📤 TX {} → Simulation node {} ({})",
                    tx_count,
                    hex::encode(&node_id[0..4]),
                    region
                );

                // Actually send the transaction
                live_network.send_transaction(*node_id, tx).await;
            }
        }

        // Generate bundles
        if rng.gen_bool(0.3) {
            bundle_count += 1;
            let bundle = create_test_bundle(&mut rng, bundle_count);

            let sim_nodes = live_network.get_simulation_nodes().await;
            if let Some((node_id, region)) = sim_nodes.get(rng.gen_range(0..sim_nodes.len().max(1)))
            {
                println!(
                    "📦 Bundle {} → Simulation node {} ({})",
                    bundle_count,
                    hex::encode(&node_id[0..4]),
                    region
                );

                live_network.send_bundle(*node_id, bundle).await;
            }
        }

        // Generate latency-sensitive transactions (sent directly to mergers)
        if rng.gen_bool(0.1) {
            latency_sensitive_count += 1;
            let mut tx = create_test_transaction(&mut rng, tx_count + 1000);
            tx.is_latency_sensitive = true;
            tx.tx.gas_price *= 5; // Higher priority

            // Send directly to a merging node
            let merging_nodes = live_network.get_merging_nodes().await;
            if let Some((node_id, region)) =
                merging_nodes.get(rng.gen_range(0..merging_nodes.len().max(1)))
            {
                println!(
                    "⚡ Latency-sensitive TX {} → Merging node {} ({})",
                    latency_sensitive_count,
                    hex::encode(&node_id[0..4]),
                    region
                );

                live_network.send_transaction(*node_id, tx).await;
            }
        }

        if tx_count % 20 == 0 {
            let elapsed = start.elapsed().as_secs();
            println!("\n⏱️  Running for {} seconds...", elapsed);

            // Show current stats
            let stats = live_network.stats.read().await;
            println!(
                "📈 Progress: {} txs, {} bundles, {} messages total",
                tx_count, bundle_count, stats.messages_sent
            );
        }

        sleep(Duration::from_millis(200)).await;
    }

    println!("\n🛑 Shutting down network...");

    // Final statistics
    let final_stats = live_network.stats.read().await;
    println!("\n📊 Final Network Statistics:");
    println!("================================");
    println!("Total transactions: {}", tx_count);
    println!("Total bundles: {}", bundle_count);
    println!("Latency-sensitive txs: {}", latency_sensitive_count);
    println!("Total messages routed: {}", final_stats.messages_sent);
    println!(
        "Transactions simulated: {}",
        final_stats.transactions_simulated
    );
    println!(
        "Partial blocks created: {}",
        final_stats.partial_blocks_sent
    );
    println!("Final blocks built: {}", final_stats.blocks_built);

    // Shutdown all nodes
    for (node_id, region, shutdown) in nodes {
        println!(
            "🔌 Stopping node {} in {}",
            hex::encode(&node_id[0..4]),
            region
        );
        let _ = shutdown.send(()).await;
    }

    // Wait for nodes to finish
    for handle in node_handles {
        let _ = handle.await;
    }

    // Cancel hub
    hub_handle.abort();

    println!("\n✅ Network shutdown complete.");
    println!("\n💡 Key Architecture Points:");
    println!("- Simulation nodes DO NOT gossip transactions between each other");
    println!("- Partial blocks are sent directly to nearby mergers (max 5)");
    println!("- Only ~10% of nodes are mergers (relay submission + archival)");
    println!("- Latency-sensitive orderflow goes directly to mergers");
    println!("- Communication scales O(N) instead of O(N²)");
}

fn create_test_transaction(rng: &mut impl Rng, nonce: u64) -> SignedTransaction {
    SignedTransaction {
        tx: Transaction {
            nonce,
            from: random_address(rng),
            to: random_address(rng),
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

fn create_test_bundle(rng: &mut impl Rng, _id: u64) -> Bundle {
    let mut txs = Vec::new();
    for i in 0..rng.gen_range(2..5) {
        txs.push(Transaction {
            nonce: i,
            from: random_address(rng),
            to: random_address(rng),
            value: rng.gen_range(100..10000),
            gas_limit: 21000,
            gas_price: rng.gen_range(50..200),
            data: vec![],
        });
    }

    Bundle {
        id: random_hash(rng),
        transactions: txs,
        reverting_tx_hashes: vec![],
        target_block: 1,
        min_timestamp: None,
        max_timestamp: None,
        is_latency_sensitive: false,
    }
}

fn random_address(rng: &mut impl Rng) -> [u8; 20] {
    let mut addr = [0u8; 20];
    rng.fill(&mut addr);
    addr
}

fn random_hash(rng: &mut impl Rng) -> [u8; 32] {
    let mut hash = [0u8; 32];
    rng.fill(&mut hash);
    hash
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// Extended NetworkHub with statistics
impl NetworkHub {
    pub async fn run_with_stats(mut self, stats: Arc<RwLock<NetworkStats>>) {
        loop {
            let mut had_messages = false;
            let node_ids: Vec<_> = self.nodes.keys().cloned().collect();

            for sender_id in node_ids {
                if let Some((_, rx)) = self.nodes.get_mut(&sender_id) {
                    if let Ok((target, msg)) = rx.try_recv() {
                        had_messages = true;

                        // Update statistics based on message type
                        match &msg {
                            Message::PartialBlock(_) => {
                                let mut s = stats.write().await;
                                s.partial_blocks_sent += 1;
                                s.messages_sent += 1;
                            }
                            Message::Transaction(_) | Message::Bundle(_) => {
                                let mut s = stats.write().await;
                                s.messages_sent += 1;
                            }
                            _ => {}
                        }

                        // Simulate network latency
                        let latency = self.calculate_latency(&sender_id, &target, &msg);
                        sleep(latency).await;

                        if let Some((tx, _)) = self.nodes.get(&target) {
                            let _ = tx.send((sender_id, msg)).await;
                        }
                    }
                }
            }

            if !had_messages {
                sleep(Duration::from_millis(10)).await;
            }
        }
    }
}
