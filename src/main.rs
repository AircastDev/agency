use agency::{async_trait, Actor, Addr, Agency, Context, Recipient};
use std::time::{Duration, Instant};
use tokio::{select, time::sleep};

#[tokio::main]
async fn main() {
    let agency = Agency::new();
    let ponger = agency.hire(Ponger::new());
    agency.hire(Pinger::new(ponger));

    agency.wait().await;
}

struct Ping(Recipient<Pong>);
struct Pong(Instant);

struct Pinger {
    ponger: Addr<Ponger>,
    hb: Instant,
}

impl Pinger {
    pub fn new(ponger: Addr<Ponger>) -> Self {
        Self {
            ponger,
            hb: Instant::now(),
        }
    }
}

#[async_trait]
impl Actor for Pinger {
    type Msg = Pong;

    async fn run(&mut self, ctx: &mut Context<Self>) {
        select! {
            msg = ctx.message() => {
                match msg {
                    Some(Pong(instant)) => {
                        let duration = instant.duration_since(self.hb);
                        println!("Pong recived after {:?}", duration);
                        self.hb = instant;
                    }
                    None => {
                        ctx.stop();
                    }
                }
            }
            _ = sleep(Duration::from_secs(5)) => {
                self.hb = Instant::now();
                self.ponger.send(Ping(ctx.address().recipient())).await;
            }
        }
    }
}

struct Ponger {}

impl Ponger {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl Actor for Ponger {
    type Msg = Ping;

    async fn run(&mut self, ctx: &mut Context<Self>) {
        match ctx.message().await {
            Some(Ping(ponger)) => {
                ponger.send(Pong(Instant::now())).await;
            }
            None => ctx.stop(),
        }
    }
}
