use super::*;

#[test]
fn reads_each_command_by_its_name() {
    for command in Command::ALL {
        assert_eq!(parse(command.name()), Ok(command));
    }
    assert_eq!(parse(" logs "), Ok(Command::Logs));
    assert_eq!(parse("stop").unwrap().control(), Some(Control::Stop));
    assert_eq!(parse("logs").unwrap().control(), None);
}

#[test]
fn an_unknown_command_says_which_there_are() {
    let err = parse("reboot").unwrap_err();
    assert_eq!(
        err,
        "Not a command: :reboot. Try :logs, :restart, :stop or :start"
    );
    // Only whole names: `st` could be two of them.
    assert!(parse("st").is_err());
}

fn typed(text: &str) -> CommandLine {
    let mut line = CommandLine::default();
    for c in text.chars() {
        line.push(c);
    }
    line
}

#[test]
fn tab_completes_and_goes_through_the_matches() {
    let mut line = typed("l");
    line.complete();
    assert_eq!(line.input, "logs");
    line.complete();
    assert_eq!(line.input, "logs");

    let mut line = typed("st");
    line.complete();
    assert_eq!(line.input, "stop");
    line.complete();
    assert_eq!(line.input, "start");
    line.complete();
    assert_eq!(line.input, "stop");

    // Typing again starts over from what is there.
    line.pop();
    line.complete();
    assert_eq!(line.input, "stop");

    let mut nothing = typed("x");
    nothing.complete();
    assert_eq!(nothing.input, "x");

    let mut empty = CommandLine::default();
    empty.complete();
    assert_eq!(empty.input, "logs");
    empty.complete();
    assert_eq!(empty.input, "restart");
}

#[test]
fn backspace_on_an_empty_line_closes_it() {
    let mut line = typed("s");
    assert!(line.pop());
    assert!(!line.pop());
}
