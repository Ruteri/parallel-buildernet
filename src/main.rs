use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::select;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, sleep};

// Core types
pub type NodeId = [u8; 32];
pub type Hash = [u8; 32];
pub type Address = [u8; 20];
pub type BlockNumber = u64;
pub type Timestamp = u64;

// Constants
const MAX_MERGER_CONNECTIONS: usize = 5; // Each simulator connects to max 5 mergers
const PARTIAL_BLOCK_INTERVAL: Duration = Duration::from_millis(100);

// Message types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Message {
    // Orderflow (NOT gossiped between simulation nodes)
    Transaction(SignedTransaction),
    Bundle(Bundle),
    PartialBlock(PartialBlock),

    // Control messages (can be gossiped)
    NodeAnnouncement(NodeInfo),
    Cancellation(CancellationRequest),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transaction {
    pub nonce: u64,
    pub from: Address,
    pub to: Address,
    pub value: u128,
    pub gas_limit: u64,
    pub gas_price: u128,
    pub data: Vec<u8>,
}

impl Transaction {
    fn hash(&self) -> Hash {
        let mut hash = [0u8; 32];
        hash[0..8].copy_from_slice(&self.nonce.to_be_bytes());
        hash[8..28].copy_from_slice(&self.from);
        hash
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedTransaction {
    pub tx: Transaction,
    pub signature: Vec<u8>,
    pub received_at: Timestamp,
    pub is_latency_sensitive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bundle {
    pub id: Hash,
    pub transactions: Vec<Transaction>,
    pub is_latency_sensitive: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartialBlock {
    pub builder_id: NodeId,
    pub transactions: Vec<Transaction>,
    pub bundles: Vec<Bundle>,
    pub total_gas_used: u64,
    pub estimated_profit: u128,
    pub conflicts: ConflictSet,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictSet {
    pub touched_accounts: HashSet<Address>,
    pub consumed_nonces: HashMap<Address, u64>,
}

impl ConflictSet {
    pub fn new() -> Self {
        Self {
            touched_accounts: HashSet::new(),
            consumed_nonces: HashMap::new(),
        }
    }

    pub fn conflicts_with(&self, other: &ConflictSet) -> bool {
        // Check account conflicts
        if !self.touched_accounts.is_disjoint(&other.touched_accounts) {
            return true;
        }

        // Check nonce conflicts
        for (addr, nonce) in &self.consumed_nonces {
            if let Some(other_nonce) = other.consumed_nonces.get(addr) {
                if nonce >= other_nonce {
                    return true;
                }
            }
        }

        false
    }

    fn merge(&mut self, other: &ConflictSet) {
        self.touched_accounts.extend(&other.touched_accounts);
        for (addr, nonce) in &other.consumed_nonces {
            self.consumed_nonces
                .entry(*addr)
                .and_modify(|n| *n = (*n).max(*nonce))
                .or_insert(*nonce);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CancellationRequest {
    pub tx_hash: Option<Hash>,
    pub bundle_id: Option<Hash>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: NodeId,
    pub role: NodeRole,
    pub region: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum NodeRole {
    Simulation,
    Merging,
}

// Network transport
pub struct NetworkTransport {
    tx: mpsc::Sender<(NodeId, Message)>,
    rx: Arc<tokio::sync::Mutex<mpsc::Receiver<(NodeId, Message)>>>,
}

impl NetworkTransport {
    pub fn new(
        _id: NodeId,
    ) -> (
        Self,
        mpsc::Sender<(NodeId, Message)>,
        mpsc::Receiver<(NodeId, Message)>,
    ) {
        let (tx_in, rx_in) = mpsc::channel(1000);
        let (tx_out, rx_out) = mpsc::channel(1000);

        let transport = Self {
            tx: tx_out,
            rx: Arc::new(tokio::sync::Mutex::new(rx_in)),
        };

        (transport, tx_in, rx_out)
    }

    async fn send(&self, peer_id: NodeId, msg: Message) {
        let _ = self.tx.send((peer_id, msg)).await;
    }

    async fn recv(&self) -> Option<(NodeId, Message)> {
        self.rx.lock().await.recv().await
    }
}

// Node configuration
#[derive(Clone)]
pub struct NodeConfig {
    pub id: NodeId,
    pub role: NodeRole, // Fixed at deployment time
    pub region: String,
}

// Simulation Node
pub struct SimulationNode {
    id: NodeId,
    region: String,
    transport: NetworkTransport,
    shutdown: mpsc::Receiver<()>,

    // State
    mergers: Arc<RwLock<HashMap<NodeId, NodeInfo>>>,
    pending_txs: Arc<RwLock<Vec<SignedTransaction>>>,
    pending_bundles: Arc<RwLock<Vec<Bundle>>>,
    cancelled: Arc<RwLock<HashSet<Hash>>>,
}

impl SimulationNode {
    pub async fn new(
        config: NodeConfig,
        transport: NetworkTransport,
        shutdown: mpsc::Receiver<()>,
    ) -> Self {
        Self {
            id: config.id,
            region: config.region,
            transport,
            shutdown,
            mergers: Arc::new(RwLock::new(HashMap::new())),
            pending_txs: Arc::new(RwLock::new(Vec::new())),
            pending_bundles: Arc::new(RwLock::new(Vec::new())),
            cancelled: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub async fn run(mut self) {
        // Announce ourselves
        let announcement = NodeInfo {
            id: self.id,
            role: NodeRole::Simulation,
            region: self.region.clone(),
        };
        self.broadcast_announcement(announcement).await;

        let mut block_timer = interval(PARTIAL_BLOCK_INTERVAL);

        loop {
            select! {
                Some((sender, msg)) = self.transport.recv() => {
                    match msg {
                        Message::Transaction(tx) => {
                            self.handle_transaction(tx).await;
                        }
                        Message::Bundle(bundle) => {
                            self.handle_bundle(bundle).await;
                        }
                        Message::NodeAnnouncement(info) => {
                            if info.role == NodeRole::Merging {
                                self.mergers.write().await.insert(info.id, info);
                            }
                        }
                        Message::Cancellation(cancel) => {
                            self.handle_cancellation(cancel).await;
                        }
                        _ => {}
                    }
                }
                _ = block_timer.tick() => {
                    self.build_and_send_partial_block().await;
                }
                _ = self.shutdown.recv() => {
                    break;
                }
            }
        }
    }

    async fn handle_transaction(&self, tx: SignedTransaction) {
        let hash = tx.tx.hash();

        // Skip if cancelled
        if self.cancelled.read().await.contains(&hash) {
            return;
        }

        // Add to pending (in real impl, would simulate first)
        self.pending_txs.write().await.push(tx);
    }

    async fn handle_bundle(&self, bundle: Bundle) {
        if self.cancelled.read().await.contains(&bundle.id) {
            return;
        }

        self.pending_bundles.write().await.push(bundle);
    }

    async fn handle_cancellation(&self, cancel: CancellationRequest) {
        let mut cancelled = self.cancelled.write().await;

        if let Some(hash) = cancel.tx_hash {
            cancelled.insert(hash);
            self.pending_txs
                .write()
                .await
                .retain(|tx| tx.tx.hash() != hash);
        }

        if let Some(id) = cancel.bundle_id {
            cancelled.insert(id);
            self.pending_bundles.write().await.retain(|b| b.id != id);
        }

        // Gossip cancellation to other nodes
        self.gossip_cancellation(cancel).await;
    }

    async fn build_and_send_partial_block(&self) {
        let mut txs = self.pending_txs.write().await;
        let mut bundles = self.pending_bundles.write().await;

        if txs.is_empty() && bundles.is_empty() {
            return;
        }

        // Build conflict-free set
        let mut conflicts = ConflictSet::new();
        let mut included_txs = Vec::new();
        let mut included_bundles = Vec::new();
        let mut gas_used = 0u64;
        let mut profit = 0u128;

        // Add transactions that don't conflict
        for tx in txs.drain(..) {
            let mut tx_conflicts = ConflictSet::new();
            tx_conflicts.touched_accounts.insert(tx.tx.from);
            tx_conflicts.touched_accounts.insert(tx.tx.to);
            tx_conflicts.consumed_nonces.insert(tx.tx.from, tx.tx.nonce);

            if !conflicts.conflicts_with(&tx_conflicts) {
                conflicts.merge(&tx_conflicts);
                gas_used += tx.tx.gas_limit;
                profit += tx.tx.gas_price * tx.tx.gas_limit as u128;
                included_txs.push(tx.tx);
            }
        }

        // Add bundles that don't conflict
        for bundle in bundles.drain(..) {
            let mut bundle_conflicts = ConflictSet::new();
            for tx in &bundle.transactions {
                bundle_conflicts.touched_accounts.insert(tx.from);
                bundle_conflicts.touched_accounts.insert(tx.to);
            }

            if !conflicts.conflicts_with(&bundle_conflicts) {
                conflicts.merge(&bundle_conflicts);
                for tx in &bundle.transactions {
                    gas_used += tx.gas_limit;
                    profit += tx.gas_price * tx.gas_limit as u128;
                }
                included_bundles.push(bundle);
            }
        }

        let partial_block = PartialBlock {
            builder_id: self.id,
            transactions: included_txs,
            bundles: included_bundles,
            total_gas_used: gas_used,
            estimated_profit: profit,
            conflicts,
        };

        // Send to nearby mergers (max 5)
        let mergers = self.get_nearby_mergers().await;
        for merger_id in mergers {
            self.transport
                .send(merger_id, Message::PartialBlock(partial_block.clone()))
                .await;
        }
    }

    async fn get_nearby_mergers(&self) -> Vec<NodeId> {
        let mergers = self.mergers.read().await;
        let mut merger_list: Vec<_> = mergers
            .iter()
            .filter(|(_, info)| info.region == self.region) // Prefer same region
            .map(|(id, _)| *id)
            .collect();

        // If not enough in same region, add from other regions
        if merger_list.len() < MAX_MERGER_CONNECTIONS {
            let other_region: Vec<_> = mergers
                .iter()
                .filter(|(_, info)| info.region != self.region)
                .map(|(id, _)| *id)
                .collect();

            merger_list.extend(other_region);
        }

        merger_list.truncate(MAX_MERGER_CONNECTIONS);
        merger_list
    }

    async fn broadcast_announcement(&self, announcement: NodeInfo) {
        // Simple broadcast (in real impl would be more sophisticated)
        self.transport
            .send([0u8; 32], Message::NodeAnnouncement(announcement))
            .await;
    }

    async fn gossip_cancellation(&self, cancel: CancellationRequest) {
        // Gossip to a few random nodes
        self.transport
            .send([0u8; 32], Message::Cancellation(cancel))
            .await;
    }
}

// Merging Node
pub struct MergingNode {
    id: NodeId,
    region: String,
    transport: NetworkTransport,
    shutdown: mpsc::Receiver<()>,

    // State
    partial_blocks: Arc<RwLock<Vec<PartialBlock>>>,
    latency_sensitive_txs: Arc<RwLock<Vec<SignedTransaction>>>,
    latency_sensitive_bundles: Arc<RwLock<Vec<Bundle>>>,
    other_mergers: Arc<RwLock<HashMap<NodeId, NodeInfo>>>,
}

impl MergingNode {
    pub async fn new(
        config: NodeConfig,
        transport: NetworkTransport,
        shutdown: mpsc::Receiver<()>,
    ) -> Self {
        Self {
            id: config.id,
            region: config.region,
            transport,
            shutdown,
            partial_blocks: Arc::new(RwLock::new(Vec::new())),
            latency_sensitive_txs: Arc::new(RwLock::new(Vec::new())),
            latency_sensitive_bundles: Arc::new(RwLock::new(Vec::new())),
            other_mergers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn run(mut self) {
        // Announce ourselves
        let announcement = NodeInfo {
            id: self.id,
            role: NodeRole::Merging,
            region: self.region.clone(),
        };
        self.broadcast_announcement(announcement).await;

        let mut block_timer = interval(Duration::from_secs(1));

        loop {
            select! {
                Some((sender, msg)) = self.transport.recv() => {
                    match msg {
                        Message::PartialBlock(partial) => {
                            self.handle_partial_block(partial).await;
                        }
                        Message::Transaction(tx) if tx.is_latency_sensitive => {
                            self.latency_sensitive_txs.write().await.push(tx);
                        }
                        Message::Bundle(bundle) if bundle.is_latency_sensitive => {
                            self.latency_sensitive_bundles.write().await.push(bundle);
                        }
                        Message::NodeAnnouncement(info) => {
                            if info.role == NodeRole::Merging && info.id != self.id {
                                self.other_mergers.write().await.insert(info.id, info);
                            }
                        }
                        _ => {}
                    }
                }
                _ = block_timer.tick() => {
                    self.build_and_submit_block().await;
                }
                _ = self.shutdown.recv() => {
                    break;
                }
            }
        }
    }

    async fn handle_partial_block(&self, partial: PartialBlock) {
        self.partial_blocks.write().await.push(partial);
    }

    async fn build_and_submit_block(&self) {
        let mut partials = std::mem::take(&mut *self.partial_blocks.write().await);
        let mut ls_txs = std::mem::take(&mut *self.latency_sensitive_txs.write().await);
        let mut ls_bundles = std::mem::take(&mut *self.latency_sensitive_bundles.write().await);

        if partials.is_empty() && ls_txs.is_empty() && ls_bundles.is_empty() {
            return;
        }

        // Sort partial blocks by profit density
        partials.sort_by_key(|p| {
            if p.total_gas_used == 0 {
                0
            } else {
                (p.estimated_profit / p.total_gas_used as u128) as i128
            }
        });
        partials.reverse();

        // Merge non-conflicting partial blocks
        let mut final_conflicts = ConflictSet::new();
        let mut final_txs = Vec::new();
        let mut total_profit = 0u128;

        for partial in partials {
            if !final_conflicts.conflicts_with(&partial.conflicts) {
                final_conflicts.merge(&partial.conflicts);
                final_txs.extend(partial.transactions);
                total_profit += partial.estimated_profit;
            }
        }

        // Add latency-sensitive transactions
        for tx in ls_txs {
            let mut tx_conflicts = ConflictSet::new();
            tx_conflicts.touched_accounts.insert(tx.tx.from);
            tx_conflicts.touched_accounts.insert(tx.tx.to);
            tx_conflicts.consumed_nonces.insert(tx.tx.from, tx.tx.nonce);

            if !final_conflicts.conflicts_with(&tx_conflicts) {
                final_conflicts.merge(&tx_conflicts);
                total_profit += tx.tx.gas_price * tx.tx.gas_limit as u128;
                final_txs.push(tx.tx);
            }
        }

        println!(
            "Merger {} built block with {} txs (profit: {})",
            hex::encode(&self.id[0..4]),
            final_txs.len(),
            total_profit
        );

        // Submit to relays (simulated)
        self.submit_to_relays(final_txs).await;
    }

    async fn submit_to_relays(&self, _txs: Vec<Transaction>) {
        println!(
            "Merger {} submitting block to relays",
            hex::encode(&self.id[0..4])
        );
        // In real implementation, would submit to actual relays
    }

    async fn broadcast_announcement(&self, announcement: NodeInfo) {
        self.transport
            .send([0u8; 32], Message::NodeAnnouncement(announcement))
            .await;
    }
}

// Factory function to create nodes based on role
pub async fn create_node(
    config: NodeConfig,
    transport: NetworkTransport,
    shutdown: mpsc::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match config.role {
        NodeRole::Simulation => {
            let node = SimulationNode::new(config, transport, shutdown).await;
            node.run().await;
        }
        NodeRole::Merging => {
            let node = MergingNode::new(config, transport, shutdown).await;
            node.run().await;
        }
    }
    Ok(())
}

// Network hub for message routing
pub struct NetworkHub {
    pub nodes: HashMap<
        NodeId,
        (
            mpsc::Sender<(NodeId, Message)>,
            mpsc::Receiver<(NodeId, Message)>,
        ),
    >,
}

impl NetworkHub {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    pub fn register_node(
        &mut self,
        id: NodeId,
        tx: mpsc::Sender<(NodeId, Message)>,
        rx: mpsc::Receiver<(NodeId, Message)>,
    ) {
        self.nodes.insert(id, (tx, rx));
    }

    pub async fn run(mut self) {
        loop {
            let node_ids: Vec<_> = self.nodes.keys().cloned().collect();

            for sender_id in node_ids {
                if let Some((_, rx)) = self.nodes.get_mut(&sender_id) {
                    if let Ok((target, msg)) = rx.try_recv() {
                        // Broadcast or direct send
                        if target == [0u8; 32] {
                            // Broadcast
                            for (id, (tx, _)) in &self.nodes {
                                if *id != sender_id {
                                    let _ = tx.send((sender_id, msg.clone())).await;
                                }
                            }
                        } else {
                            // Direct send
                            if let Some((tx, _)) = self.nodes.get(&target) {
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

// Helper functions
pub fn generate_node_id() -> NodeId {
    let mut id = [0u8; 32];
    thread_rng().fill(&mut id);
    id
}

pub mod hex {
    pub fn encode(data: &[u8]) -> String {
        data.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

// Main function for when compiled as a binary
#[cfg(not(feature = "library"))]
#[tokio::main]
async fn main() {
    println!("Distributed Block Builder - Simplified");
    println!("=====================================");
    println!("Architecture: Fixed role assignment at deployment");
    println!("- ~90% Simulation nodes");
    println!("- ~10% Merging nodes");
    println!("- Dynamic discovery of simulation→merger connections");
    println!("\nRun `cargo run --example simple_network` to see the protocol in action");
}
