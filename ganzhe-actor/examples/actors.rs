use std::{cell::RefCell, rc::Rc, time::Duration};

use ganzhe_actor::{Actor, ActorSpawner, Handler};

fn main() -> Result<(), anyhow::Error> {
    let rt = ganzhe_rt::Builder::new().build()?;
    rt.block_on(async move {
        ganzhe_rt::spawn_to_random_local_scheduler(|| async move {
            let actor = PingPong {};
            let addr = ActorSpawner::new().spawn(actor);
            let counter = Rc::new(RefCell::new(0 as usize));
            let counter_2 = counter.clone();

            ganzhe_rt::spawn_local(async move {
                loop {
                    let _ = addr.send(Ping(0)).await.unwrap();
                    let mut counter = counter.borrow_mut();
                    *counter += 1;
                }
            });

            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let count = {
                    let mut counter = counter_2.borrow_mut();
                    let count: usize = *counter;
                    *counter = 0;
                    count
                };
                println!("{count}");
            }
        });
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    Ok(())
}

struct PingPong {}

impl Actor for PingPong {}

impl Handler<Ping> for PingPong {
    type Resp = Pong;

    fn can_fast_handle(_msg: &Ping, _ctx: &ganzhe_actor::Context<Self>) -> bool {
        true
    }

    fn fast_handle(msg: Ping, _ctx: &ganzhe_actor::Context<Self>) -> Self::Resp {
        Pong(msg.0)
    }
}

struct Ping(usize);

struct Pong(usize);
