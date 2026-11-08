use super::{memory_bytes, os_name, parse_disk};

#[test]
fn parses_linux_facts_and_rejects_bad_values() {
    assert_eq!(
        os_name("ID=ubuntu\nPRETTY_NAME=\"Ubuntu Test\"\n").expect("os"),
        "Ubuntu Test"
    );
    assert!(os_name("ID=linux\n").is_err());
    let memory = "MemTotal: 1024 kB\nMemAvailable: 512 kB\n";
    assert_eq!(memory_bytes(memory, "MemTotal").expect("total"), 1_048_576);
    assert_eq!(
        memory_bytes(memory, "MemAvailable").expect("available"),
        524_288
    );
    assert!(memory_bytes("MemTotal: 9 MB\n", "MemTotal").is_err());
    assert!(memory_bytes("MemTotal: 18446744073709551615 kB\n", "MemTotal").is_err());
    assert_eq!(
        parse_disk(
            "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/a 200 20 180 10% /\n"
        )
        .expect("disk"),
        (204_800, 184_320)
    );
    assert!(parse_disk("Filesystem\n").is_err());
}
