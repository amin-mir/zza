use std::time::Duration;

use zza::{sleep, Executor};

fn main() {
    let executor = Executor::new();

    executor.spawn(async {
        println!("spawning the sleep future");
        sleep(Duration::from_secs(2)).await;
        println!("sleep ended!");
    });

    executor.run();
}
