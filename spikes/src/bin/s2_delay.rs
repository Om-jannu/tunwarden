// Poll every 100 ms in one long-lived process; report when each API first sees the port.
use std::time::{Duration, Instant};
fn main() {
    let port: u16 = std::env::args().nth(1).unwrap().parse().unwrap();
    let t0 = Instant::now();
    let (mut a, mut b) = (None, None);
    while t0.elapsed() < Duration::from_secs(20) && (a.is_none() || b.is_none()) {
        if a.is_none() && listeners::get_all().unwrap().iter().any(|l| l.socket.port() == port) { a = Some(t0.elapsed()); }
        if b.is_none() && listeners::get_process_by_port(port, listeners::Protocol::TCP).is_ok() { b = Some(t0.elapsed()); }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("get_all first saw it at {a:?}, get_process_by_port at {b:?}");
}
