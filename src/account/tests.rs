    use super::*;
    #[tokio::test]
    async fn session_lifecycle() {
        let db = Accounts::open(":memory:").await.unwrap();
        db.create_user("tester", "long-test-password".into())
            .await
            .unwrap();
        assert!(db
            .login("tester".into(), "incorrect".into())
            .await
            .unwrap()
            .is_none());
        let token = db
            .login("tester".into(), "long-test-password".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(db.user(&token).await.unwrap().as_deref(), Some("tester"));
        assert!(db.user("").await.unwrap().is_none());
        assert!(db.user(&"a".repeat(64)).await.unwrap().is_none());
        let ticket = db.ticket(&token, "relay-1").await.unwrap().unwrap();
        assert!(!db.redeem(&ticket, "relay-2").await.unwrap());
        assert!(db.redeem(&ticket, "relay-1").await.unwrap());
        assert!(!db.redeem(&ticket, "relay-1").await.unwrap());
        let ticket = db.ticket(&token, "relay-1").await.unwrap().unwrap();
        db.revoke(&token).await.unwrap();
        assert!(!db.redeem(&ticket, "relay-1").await.unwrap());
        assert!(db.user(&token).await.unwrap().is_none());
        let token = db
            .login("tester".into(), "long-test-password".into())
            .await
            .unwrap()
            .unwrap();
        sqlx::query("UPDATE account_sessions SET expires=0")
            .execute(&db.pool)
            .await
            .unwrap();
        assert!(db.user(&token).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn admin_commands() {
        let db = Accounts::open(":memory:").await.unwrap();
        assert!(db.create_user("bad name", "long-test-password".into()).await.is_err());
        assert!(db.create_user("short", "short".into()).await.is_err());
        db.create_user("alice", "long-test-password".into()).await.unwrap();
        db.create_user("bob", "long-test-password".into()).await.unwrap();
        assert!(db.create_user("alice", "long-test-password".into()).await.is_err());
        let token = db.login("alice".into(), "long-test-password".into()).await.unwrap().unwrap();
        let ticket = db.ticket(&token, "relay-1").await.unwrap().unwrap();
        let listed = db.list_users().await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!((listed[0].name.as_str(), listed[0].enabled, listed[0].sessions), ("alice", true, 1));
        assert_eq!((listed[1].name.as_str(), listed[1].sessions), ("bob", 0));

        assert!(db.set_password("nobody", "long-test-password".into()).await.is_err());
        assert!(db.set_password("alice", "short".into()).await.is_err());
        db.set_password("alice", "another-long-password".into()).await.unwrap();
        assert!(db.user(&token).await.unwrap().is_none());
        assert!(!db.redeem(&ticket, "relay-1").await.unwrap());
        assert!(db.login("alice".into(), "long-test-password".into()).await.unwrap().is_none());
        let token = db.login("alice".into(), "another-long-password".into()).await.unwrap().unwrap();

        assert!(db.set_enabled("nobody", false).await.is_err());
        db.set_enabled("alice", false).await.unwrap();
        assert!(db.user(&token).await.unwrap().is_none());
        assert!(db.login("alice".into(), "another-long-password".into()).await.unwrap().is_none());
        assert!(!db.list_users().await.unwrap()[0].enabled);
        db.set_enabled("alice", true).await.unwrap();
        assert!(db.login("alice".into(), "another-long-password".into()).await.unwrap().is_some());

        sqlx::query("INSERT INTO wol_devices(owner,id,seen,session,peers) VALUES('alice','dev',0,x'00','[]')")
            .execute(&db.pool).await.unwrap();
        assert!(db.delete_user("nobody").await.is_err());
        db.delete_user("alice").await.unwrap();
        let listed = db.list_users().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "bob");
        let leftover: i64 = sqlx::query("SELECT COUNT(*) AS n FROM wol_devices WHERE owner='alice'")
            .fetch_one(&db.pool).await.unwrap().get("n");
        assert_eq!(leftover, 0);
        assert!(db.login("alice".into(), "another-long-password".into()).await.unwrap().is_none());
    }
