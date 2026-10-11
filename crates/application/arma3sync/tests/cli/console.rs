//! The interactive console, driven through standard input.

use a3s_types::ProtocolType;

use crate::support::{
    Env, HttpServer, assert_exit, http_protocol, server_info, write_repository_content,
};

#[test]
fn console_creates_and_deletes_a_repository() {
    let env = Env::new();
    std::fs::create_dir_all(env.path("repo")).expect("mkdir");
    let folder = env.path("repo").to_string_lossy().into_owned();
    // Name (a forbidden one first), protocol (an unknown one first), port, login, password,
    // URL with a scheme, main folder.
    let input = format!(
        "new\nbad/name\nconsole repo\ngopher\nhttps\nmaybe\nyes\n8443\nanonymous\n\n\
         HTTPS://example.org/repo\n{folder}\nLIST\nVERSION\nQUIT\n"
    );
    assert_exit(&env.console(&input), 0);

    let stored = env.repository("console repo");
    assert_eq!(stored.protocol.protocol_type, ProtocolType::Https);
    assert_eq!(stored.protocol.url, "example.org/repo");
    assert_eq!(stored.protocol.port, "8443");
    assert!(stored.protocol.validate_ssl_certificate);
    assert_eq!(stored.protocol.connection_timeout.as_deref(), Some("0"));
    assert_eq!(stored.protocol.read_timeout.as_deref(), Some("0"));
    assert_eq!(stored.path.as_deref(), Some(folder.as_str()));

    assert_exit(&env.console("DELETE\n\nconsole repo\nquit\n"), 0);
    assert!(env.data().repository_names().is_empty());
}

#[test]
fn console_build_stores_the_settings_and_builds() {
    let env = Env::new();
    let server = HttpServer::serve(&env.path("repo"));
    write_repository_content(&env.path("repo"));
    env.add_repository("admin", &http_protocol(server.port), |_| {});
    let folder = env.path("repo").to_string_lossy().into_owned();
    let excluded = env.path("repo/readme.txt").to_string_lossy().into_owned();
    // Name, main folder, connections (out of range first), compressed, partial transfer,
    // excluded files (a missing one first), excluded folders.
    let input =
        format!("BUILD\nadmin\n{folder}\n11\n3\nno\nno\n{folder}/missing\n{excluded}\n\n\nQUIT\n");
    assert_exit(&env.console(&input), 0);

    let stored = env.repository("admin");
    assert_eq!(stored.path.as_deref(), Some(folder.as_str()));
    assert_eq!(stored.number_of_connections, 3);
    assert!(!stored.compressed);
    assert!(!stored.use_partial_file_transfer());
    assert_eq!(
        stored
            .excluded_files_from_build
            .into_iter()
            .collect::<Vec<_>>(),
        [excluded]
    );
    assert!(stored.excluded_folders_from_sync.is_empty());
    assert_eq!(stored.revision, server_info(&env.path("repo")).revision);

    // The console's check of the built repository succeeds.
    assert_exit(&env.console("CHECK\nadmin\nQUIT\n"), 0);
}

#[test]
fn console_fails_on_unknown_commands_and_truncated_input() {
    let env = Env::new();
    assert_exit(&env.console("BOGUS\nQUIT\n"), 1);
    assert_exit(&env.console("NEW\nhalf done\nHTTP\n"), 1);
    assert!(env.data().repository_names().is_empty());
    assert_exit(&env.console("BUILD\nmissing\nQUIT\n"), 1);
    // The end of the input ends the console like QUIT.
    assert_exit(&env.console("COMMANDS\nVERSION\n"), 0);
    assert_exit(&env.console(""), 0);
}
