# zza
Zan, Zendegi, Azadi. Named in honor of brave women of Iran! It means "women, life and freedom". `zza` is a Rust async runtime for educational purposes only!

## TODO
* Enable github actions to run tests.
* Enable normal program finish, so when the main future finishes
  the program should exit.
* move the creation of done, interrupt, and sleep channels to a single place
  in sleep reactor so all tests create those channels the same way.
* read what-the-async and other async-related articles to get inspiration for IO reactor.

## Features
* Graceful shutdown.
* Program finishes gracefully when all the Futures are complete.

## Credits
This practice project is highly inspired by the following:
* https://github.com/tokio-rs/website/tree/master/tutorial-code/mini-tokio
* https://github.com/conradludgate/what-the-async
* https://github.com/DomWilliams0/name-needed
* https://github.com/cfsamson/examples-futures
