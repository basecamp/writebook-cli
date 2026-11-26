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

#[test]
fn an_inline_tests_module_over_two_hundred_lines_has_its_own_file() {
    let long = format!(
        "fn app() {{}}\n\n#[cfg(test)]\nmod tests {{\n{}}}\n",
        "    fn a() {}\n".repeat(198)
    );
    let short = format!(
        "fn app() {{}}\n\n#[cfg(test)]\nmod tests {{\n{}}}\n",
        "    fn a() {}\n".repeat(197)
    );
    let not_tests = format!("mod parts {{\n{}}}\n", "    fn a() {}\n".repeat(300));

    assert_eq!(
        violations(
            &[
                ("src/long.rs", &long),
                ("src/short.rs", &short),
                ("src/parts.rs", &not_tests)
            ],
            a_long_tests_module_has_its_own_file
        ),
        ["src/long.rs:4: 201 lines"]
    );
}
