use super::*;
use crate::lint::tree::violations;

#[test]
fn a_test_may_sleep_only_in_a_fake_servers_route_or_while_it_waits_on_a_state() {
    let source = "fn slow() {
             Server::start(|request| { sleep(TICK); reply(200, \"\") });
             Server::start(|_: &Request| { sleep(TICK); reply(200, \"\") });
             run(|| sleep(TICK));
         }
         fn eventually(done: impl Fn() -> bool) {
             while !done() { sleep(TICK); }
         }";

    assert_eq!(
        violations(&[("tests/a.rs", source)], no_test_sleeps),
        ["tests/a.rs:4: sleep"]
    );
}

#[test]
fn a_test_may_not_read_a_clock_even_in_a_route() {
    let source = "fn timed() {
             Server::start(|request| { let waited = started.elapsed(); reply(200, \"\") });
             let took = started.elapsed();
         }";

    assert_eq!(
        violations(&[("tests/a.rs", source)], no_test_reads_how_long_something_took),
        ["tests/a.rs:2: elapsed", "tests/a.rs:3: elapsed"]
    );
}
