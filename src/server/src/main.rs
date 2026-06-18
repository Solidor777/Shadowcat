fn main() {
    let status = shadowcat::health::HealthStatus::ok(false);
    println!(
        "shadowcat {} (db_connected={})",
        status.status, status.db_connected
    );
}
