//! Windows command-line and PowerShell quoting.

use a3s_foundation::command_line::{
    encode_powershell_command, quote_powershell, quote_windows_arg, windows_args,
};
use base64::Engine as _;

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|&arg| arg.to_owned()).collect()
}

#[test]
fn windows_arguments_are_quoted_for_command_line_to_argv() {
    assert_eq!(quote_windows_arg("-x"), "-x");
    assert_eq!(
        quote_windows_arg(r"-mod=C:\My Mods\@ace;"),
        r#""-mod=C:\My Mods\@ace;""#
    );
    assert_eq!(quote_windows_arg(r#"a"b"#), r#""a\"b""#);
    assert_eq!(quote_windows_arg(r"C:\x y\"), r#""C:\x y\\""#);
    assert_eq!(quote_windows_arg(""), r#""""#);

    assert_eq!(
        windows_args(&strings(&["-mod=@CBA_A3;@ace", "-noSplash"])),
        "-mod=@CBA_A3;@ace -noSplash"
    );
    assert_eq!(
        windows_args(&strings(&["-mod=C:\\My Mods\\@ace\\", "x\"y"])),
        r#""-mod=C:\My Mods\@ace\\" "x\"y""#
    );
    assert_eq!(windows_args(&[]), "");
}

#[test]
fn powershell_literals_double_every_single_quote() {
    assert_eq!(quote_powershell("O'Neil"), "'O''Neil'");
    assert_eq!(quote_powershell("a\u{2019}b"), "'a\u{2019}\u{2019}b'");
    assert_eq!(
        quote_powershell("\u{2018}\u{201A}\u{201B}"),
        "'\u{2018}\u{2018}\u{201A}\u{201A}\u{201B}\u{201B}'"
    );
}

#[test]
fn encoded_command_is_utf16le_base64() {
    let script = "Write-Host 'é'";
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encode_powershell_command(script))
        .unwrap();
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    assert_eq!(String::from_utf16(&units).unwrap(), script);
}
