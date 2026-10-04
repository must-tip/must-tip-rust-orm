use media::{ClamAvScanner, MalwareScanner, ScanResult};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn scanner_response(response: &'static [u8]) -> Result<ScanResult, media::MediaError> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let provider = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut command = [0; 10];
        socket.read_exact(&mut command).await.unwrap();
        assert_eq!(&command, b"zINSTREAM\0");
        loop {
            let mut size = [0; 4];
            socket.read_exact(&mut size).await.unwrap();
            let size = u32::from_be_bytes(size) as usize;
            if size == 0 {
                break;
            }
            let mut data = vec![0; size];
            socket.read_exact(&mut data).await.unwrap();
        }
        socket.write_all(response).await.unwrap();
    });
    let result = ClamAvScanner::new("127.0.0.1", address.port(), 1)
        .unwrap()
        .scan("example.txt", b"safe")
        .await;
    provider.await.unwrap();
    result
}

#[tokio::test]
async fn uploads_accept_only_an_explicit_clean_scanner_response() {
    assert_eq!(
        scanner_response(b"stream: OK\0").await.unwrap(),
        ScanResult::Clean
    );
    assert_eq!(
        scanner_response(b"stream: Eicar-Test-Signature FOUND\0")
            .await
            .unwrap(),
        ScanResult::Infected {
            signature: "Eicar-Test-Signature".into()
        }
    );
    assert!(scanner_response(b"stream: NOT OK\0").await.is_err());
    assert!(scanner_response(b"stream: OK").await.is_err());
    assert!(
        scanner_response(b"provider unavailable ERROR\0")
            .await
            .is_err()
    );
}
