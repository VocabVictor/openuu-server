    use super::*;
    #[tokio::test]
    async fn client_api_and_revocation() {
        let db = Arc::new(Accounts::open(":memory:").await.unwrap());
        db.create_user("apitest", "test-only-long-password".into())
            .await
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(
            axum::Server::from_tcp(listener)
                .unwrap()
                .serve(router(db).into_make_service_with_connect_info::<SocketAddr>()),
        );
        let client = reqwest::Client::new();
        let base = format!("http://{}", addr);
        assert_eq!(
            client
                .post(format!("{base}/api/currentUser"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            client
                .get(format!("{base}/api/login-options"))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap(),
            json!([])
        );
        let response = client
            .post(format!("{base}/api/login"))
            .body(json!({"username":"apitest","password":"test-only-long-password"}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let login: Value = response.json().await.unwrap();
        let token = login["access_token"].as_str().unwrap();
        assert_eq!(login["type"], "access_token");
        assert_eq!(
            client
                .post(format!("{base}/api/currentUser"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .post(format!("{base}/api/relay-ticket"))
                .bearer_auth(token)
                .json(&json!({"uuid":"test-relay"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .post(format!("{base}/api/logout"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .post(format!("{base}/api/currentUser"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for _ in 0..10 {
            let _ = client
                .post(format!("{base}/api/login"))
                .body("invalid")
                .send()
                .await
                .unwrap();
        }
        assert_eq!(
            client
                .post(format!("{base}/api/login"))
                .body("invalid")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        server.abort();
    }
