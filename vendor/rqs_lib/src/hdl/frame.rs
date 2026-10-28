use std::io::{Error, ErrorKind};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, timeout_at};

pub(super) const MAX_FRAME_LENGTH: usize = 5 * 1024 * 1024;
const PARTIAL_FRAME_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Retains consumed bytes when a channel action wins the request's select!.
/// read_exact with a stack-local prefix loses bytes if that future is cancelled.
#[derive(Debug, Default)]
pub(super) struct FrameReader {
    prefix: [u8; 4],
    prefix_read: usize,
    body: Vec<u8>,
    body_read: usize,
    last_progress: Option<Instant>,
}

impl FrameReader {
    pub async fn read(
        &mut self,
        stream: &mut (impl AsyncRead + Unpin),
    ) -> std::io::Result<Vec<u8>> {
        while self.prefix_read < self.prefix.len() {
            self.prefix_read += read_part(
                stream,
                &mut self.prefix[self.prefix_read..],
                &mut self.last_progress,
            )
            .await?;
        }
        if self.body.is_empty() {
            let length = u32::from_be_bytes(self.prefix) as usize;
            if length == 0 || length > MAX_FRAME_LENGTH {
                return Err(Error::new(ErrorKind::InvalidData, "Invalid frame length"));
            }
            self.body.resize(length, 0);
        }
        while self.body_read < self.body.len() {
            self.body_read += read_part(
                stream,
                &mut self.body[self.body_read..],
                &mut self.last_progress,
            )
            .await?;
        }
        self.prefix_read = 0;
        self.body_read = 0;
        self.last_progress = None;
        Ok(std::mem::take(&mut self.body))
    }
}

async fn read_part(
    stream: &mut (impl AsyncRead + Unpin),
    buffer: &mut [u8],
    last_progress: &mut Option<Instant>,
) -> std::io::Result<usize> {
    // Waiting for a new frame may include human consent. Only an unfinished
    // frame has an idle deadline, which channel traffic cannot reset.
    let count = match *last_progress {
        Some(progress) => timeout_at(progress + PARTIAL_FRAME_IDLE_TIMEOUT, stream.read(buffer))
            .await
            .map_err(|_| Error::new(ErrorKind::TimedOut, "Peer stalled during a frame"))??,
        None => stream.read(buffer).await?,
    };
    if count == 0 {
        return Err(Error::new(
            ErrorKind::UnexpectedEof,
            "Peer closed the stream",
        ));
    }
    *last_progress = Some(Instant::now());
    Ok(count)
}

/// A peer that stops reading cannot hold a transfer task in write_all forever.
pub(super) async fn write(
    stream: &mut (impl AsyncWrite + Unpin),
    body: &[u8],
) -> std::io::Result<()> {
    write_with_timeout(stream, body, Duration::from_secs(30)).await
}

async fn write_with_timeout(
    stream: &mut (impl AsyncWrite + Unpin),
    body: &[u8],
    limit: Duration,
) -> std::io::Result<()> {
    if body.is_empty() || body.len() > MAX_FRAME_LENGTH {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "Invalid outgoing frame length",
        ));
    }
    tokio::time::timeout(limit, async {
        stream.write_u32(body.len() as u32).await?;
        stream.write_all(body).await?;
        stream.flush().await
    })
    .await
    .map_err(|_| Error::new(ErrorKind::TimedOut, "Peer stopped reading a frame"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn fragmented_prefix_and_body_survive_cancelled_reads() {
        let (mut sender, mut receiver) = tokio::io::duplex(64);
        let mut reader = FrameReader::default();
        sender.write_all(&[0, 0]).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), reader.read(&mut receiver))
                .await
                .is_err()
        );
        assert_eq!(reader.prefix_read, 2);
        sender.write_all(&[0, 4, b'a', b'b']).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), reader.read(&mut receiver))
                .await
                .is_err()
        );
        assert_eq!(reader.body_read, 2);
        // Include another complete frame to verify boundaries and reader reset.
        sender
            .write_all(&[b'c', b'd', 0, 0, 0, 1, b'z'])
            .await
            .unwrap();
        assert_eq!(reader.read(&mut receiver).await.unwrap(), b"abcd");
        assert_eq!(reader.read(&mut receiver).await.unwrap(), b"z");
    }

    #[tokio::test]
    async fn invalid_lengths_and_truncated_frames_are_rejected() {
        for length in [0, MAX_FRAME_LENGTH as u32 + 1, u32::MAX] {
            let (mut sender, mut receiver) = tokio::io::duplex(64);
            sender.write_all(&length.to_be_bytes()).await.unwrap();
            let mut reader = FrameReader::default();
            assert_eq!(
                reader.read(&mut receiver).await.unwrap_err().kind(),
                ErrorKind::InvalidData
            );
            assert!(reader.body.is_empty());
        }
        for data in [vec![0, 0], vec![0, 0, 0, 3, b'x']] {
            let (mut sender, mut receiver) = tokio::io::duplex(64);
            sender.write_all(&data).await.unwrap();
            drop(sender);
            assert_eq!(
                FrameReader::default()
                    .read(&mut receiver)
                    .await
                    .unwrap_err()
                    .kind(),
                ErrorKind::UnexpectedEof
            );
        }
    }

    #[tokio::test]
    async fn partial_frame_timeout_survives_read_restarts() {
        let (_sender, mut receiver) = tokio::io::duplex(64);
        let mut reader = FrameReader {
            prefix_read: 2,
            last_progress: Some(Instant::now() - PARTIAL_FRAME_IDLE_TIMEOUT),
            ..Default::default()
        };
        for _ in 0..2 {
            assert_eq!(
                reader.read(&mut receiver).await.unwrap_err().kind(),
                ErrorKind::TimedOut
            );
        }
    }
    #[tokio::test]
    async fn stalled_writes_time_out_and_normal_writes_keep_the_frame_boundary() {
        let (mut sender, _blocked) = tokio::io::duplex(1);
        assert_eq!(
            write_with_timeout(&mut sender, b"hello", Duration::from_millis(20))
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::TimedOut
        );
        let (mut sender, mut receiver) = tokio::io::duplex(64);
        write(&mut sender, b"hello").await.unwrap();
        write(&mut sender, b"world").await.unwrap();
        let mut reader = FrameReader::default();
        assert_eq!(reader.read(&mut receiver).await.unwrap(), b"hello");
        assert_eq!(reader.read(&mut receiver).await.unwrap(), b"world");
    }
}
