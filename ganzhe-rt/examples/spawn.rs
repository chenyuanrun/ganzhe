use std::time::Duration;

use clap::Parser;

#[derive(Debug, clap::Parser)]
struct Args {}

fn main() -> anyhow::Result<()> {
    let _args = Args::parse();

    let rt = ganzhe_rt::Builder::new().build()?;
    rt.block_on(async {
        let mut cur_id: usize = 0;
        for id in cur_id..cur_id + 10 {
            ganzhe_rt::spawn_to_cross_scheduler(thread_reporter(id, None));
        }
        cur_id += 10;
        for id in cur_id..cur_id + 10 {
            ganzhe_rt::spawn_to_random_local_scheduler(move || thread_reporter(id, None));
        }
        cur_id += 10;

        thread_reporter(cur_id, None).await;
    });
    Ok(())
}

async fn thread_reporter(task_id: usize, times: Option<usize>) {
    let cur = std::thread::current();
    let thread_name = cur.name();
    let times = times.unwrap_or(usize::MAX);
    for _ in 0..times {
        println!("task {task_id} run in thread {thread_name:?}");
        // do some work.
        for _ in 0..1000 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
