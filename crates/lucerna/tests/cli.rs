use assert_cmd::Command;

#[test]
fn version_flag_prints_name_and_version() {
    Command::cargo_bin("luc")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout("luc 0.1.0\n");
}
