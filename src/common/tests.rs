    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn argument_names_ignore_case_and_separator() {
        let aliases = [
            "RUSTDESK-CONFIG-ALIAS-TEST",
            "RUSTDESK_CONFIG_ALIAS_TEST",
            "rustdesk-config-alias-test",
            "rustdesk_config_alias_test",
            "RustDesk_Config-Alias_Test",
        ];
        for alias in aliases {
            std::env::remove_var(alias);
        }
        for alias in aliases {
            std::env::set_var(alias, alias);
            assert_eq!(get_arg("RUSTDESK_CONFIG_ALIAS_TEST"), alias);
            std::env::remove_var(alias);
        }
        set_arg("rustdesk_config_alias_test", "normalized");
        assert_eq!(
            std::env::var("RUSTDESK-CONFIG-ALIAS-TEST").unwrap(),
            "normalized"
        );
        std::env::set_var("RUSTDESK_CONFIG_ALIAS_TEST", "inherited");
        set_arg("rustdesk-config-alias-test", "higher-priority");
        assert_eq!(get_arg("rustdesk_config_alias_test"), "higher-priority");
        std::env::remove_var("RUSTDESK-CONFIG-ALIAS-TEST");
        std::env::remove_var("RUSTDESK_CONFIG_ALIAS_TEST");
    }

    #[test]
    fn parses_bind_address() {
        assert_eq!(parse_bind_address("").unwrap(), None);
        assert_eq!(
            parse_bind_address("127.0.0.1").unwrap(),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
        assert_eq!(
            parse_bind_address("::1").unwrap(),
            Some(IpAddr::V6(Ipv6Addr::LOCALHOST))
        );
        assert!(parse_bind_address("not-an-ip").is_err());
    }

    #[hbb_common::tokio::test]
    async fn tcp_listener_uses_bind_address() {
        let bind_addr = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let listener = listen_tcp(Some(bind_addr), 0).await.unwrap();
        assert_eq!(listener.local_addr().unwrap().ip(), bind_addr);
    }

    #[test]
    fn console_addr_only_when_bind_does_not_cover_ipv4_localhost() {
        for bind_addr in [
            None,
            Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ] {
            assert_eq!(console_addr(bind_addr, 21117), None);
        }
        for bind_addr in [
            Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))),
            Some("2001:db8::1".parse().unwrap()),
            Some(IpAddr::V6(Ipv6Addr::LOCALHOST)),
        ] {
            assert_eq!(
                console_addr(bind_addr, 21117),
                Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 21117))
            );
        }
    }

    #[hbb_common::tokio::test]
    async fn console_listener_binds_ipv4_localhost() {
        let listener = listen_console(Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))), 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            listener.local_addr().unwrap().ip(),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        assert!(listen_console(None, 0).await.unwrap().is_none());
        assert!(listen_console(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)), 0)
            .await
            .unwrap()
            .is_none());
    }
