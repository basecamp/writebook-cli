use super::*;

#[test]
fn the_sleep_reader_lets_a_fake_servers_route_answer_slowly() {
    let slow = syn::parse_file(
        "fn slow() {
             Server::start(|request| { sleep(TICK); reply(200, \"\") });
             Server::start(|_: &Request| { sleep(TICK); reply(200, \"\") });
             run(|| sleep(TICK));
         }
         fn eventually(done: impl Fn() -> bool) {
             while !done() { sleep(TICK); }
         }",
    )
    .expect("the example parses");
    let mut calls = Calls {
        found: Found::new("a.rs"),
        names: &SLEEPS,
        function: None,
        routes_may_call: true,
        in_a_route: false,
    };

    calls.visit_file(&slow);

    assert_eq!(vec!["a.rs:4: sleep"], calls.found.lines);
}

#[test]
fn the_clock_reader_refuses_a_route_that_times_itself() {
    let timed = syn::parse_file(
        "fn timed() {
             Server::start(|request| { let waited = started.elapsed(); reply(200, \"\") });
             let took = started.elapsed();
         }",
    )
    .expect("the example parses");
    let mut calls = Calls {
        found: Found::new("a.rs"),
        names: &CLOCK_READINGS,
        function: None,
        routes_may_call: false,
        in_a_route: false,
    };

    calls.visit_file(&timed);

    assert_eq!(vec!["a.rs:2: elapsed", "a.rs:3: elapsed"], calls.found.lines);
}
