fn main() {
    let port: u16 = std::env::args().nth(1).unwrap().parse().unwrap();
    let all = listeners::get_all().unwrap();
    for l in all.iter().filter(|l| l.socket.port() == port) { println!("listeners: {} pid {} {} {:?}", l.socket, l.process.pid, l.process.name, l.state); }
    println!("by_port: {:?}", listeners::get_process_by_port(port, listeners::Protocol::TCP));
}
