use super::parse;

#[test]
fn reads_the_pairs_systemd_would() {
    let values = parse(
        b"# comment\n; another\n\nA=one\nexport B=\"two words\"\nC='three'\nBAD KEY=x\n9NOPE=y\nEMPTY=\nD=\n",
    );
    assert_eq!(values.get("A").map(String::as_str), Some("one"));
    assert_eq!(values.get("B").map(String::as_str), Some("two words"));
    assert_eq!(values.get("C").map(String::as_str), Some("three"));
    assert_eq!(values.get("EMPTY").map(String::as_str), Some(""));
    assert!(!values.contains_key("BAD KEY"));
    assert!(!values.contains_key("9NOPE"));
}

#[test]
fn removes_one_pair_of_quotes_and_no_expansion() {
    let values = parse(b"A=\"$HOME\"\nB='a'b'\nC=\"unclosed\n");
    assert_eq!(values.get("A").map(String::as_str), Some("$HOME"));
    assert_eq!(values.get("B").map(String::as_str), Some("a'b"));
    assert_eq!(values.get("C").map(String::as_str), Some("\"unclosed"));
}

#[test]
fn a_later_pair_wins() {
    let values = parse(b"A=one\nA=two\n");
    assert_eq!(values.get("A").map(String::as_str), Some("two"));
}
