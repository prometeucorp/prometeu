//! Reassemble bounded runtime blocks behind the existing application's byte-buffer contract.
use crate::application::ApplicationClient;
use prometeu_core::files::{FileBlock, FILE_BLOCK_BYTES, MAX_FILE_BYTES};
use serde_json::{json, Value};

pub(crate) fn read(client: &mut dyn ApplicationClient, mut args: Value) -> Result<Vec<u8>, String> {
    let args = args.as_object_mut().ok_or("file arguments are required")?;
    let mut bytes = Vec::new();
    let mut previous: Option<(u64, String)> = None;
    loop {
        args.insert("offset".into(), json!(bytes.len()));
        args.insert(
            "stamp".into(),
            json!(previous.as_ref().map(|(_, stamp)| stamp)),
        );
        let block: FileBlock =
            serde_json::from_value(client.application("read_bytes".into(), json!(args))?)
                .map_err(|error| error.to_string())?;
        let offset = bytes.len() as u64;
        if block.size > MAX_FILE_BYTES
            || block.size < offset
            || block.stamp.is_empty()
            || previous
                .as_ref()
                .is_some_and(|(size, stamp)| *size != block.size || *stamp != block.stamp)
            || block.data.len() as u64 != (block.size - offset).min(FILE_BLOCK_BYTES as u64)
        {
            return Err("invalid file block".into());
        }
        bytes.extend_from_slice(&block.data);
        if bytes.len() as u64 == block.size {
            return Ok(bytes);
        }
        previous = Some((block.size, block.stamp));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Client {
        replies: VecDeque<Result<Value, String>>,
        requests: Vec<Value>,
    }
    impl ApplicationClient for Client {
        fn application(&mut self, command: String, args: Value) -> Result<Value, String> {
            assert_eq!(command, "read_bytes");
            self.requests.push(args);
            self.replies.pop_front().expect("no extra request")
        }
    }
    fn block(size: u64, stamp: &str, data: Vec<u8>) -> Result<Value, String> {
        Ok(json!({"size":size,"stamp":stamp,"data":data}))
    }
    #[test]
    fn blocks_preserve_binary_values_and_return_nothing_on_interrupted_or_changed_reads() {
        let prefix: Vec<_> = (0..FILE_BLOCK_BYTES).map(|i| (i % 256) as u8).collect();
        let size = FILE_BLOCK_BYTES as u64 + 3;
        let mut client = Client {
            replies: VecDeque::from([
                block(size, "revision", prefix.clone()),
                block(size, "revision", vec![0, 255, 128]),
            ]),
            requests: vec![],
        };
        let body = read(&mut client, json!({"id":"project","rel":"image.png"})).unwrap();
        assert_eq!(&body[..FILE_BLOCK_BYTES], &prefix);
        assert_eq!(&body[FILE_BLOCK_BYTES..], &[0, 255, 128]);
        assert_eq!(
            client.requests,
            [
                json!({"id":"project","rel":"image.png","offset":0,"stamp":null}),
                json!({"id":"project","rel":"image.png","offset":FILE_BLOCK_BYTES,"stamp":"revision"})
            ]
        );
        for last in [
            Err("disconnected".into()),
            block(size, "changed", vec![0; 3]),
            block(size + 1, "revision", vec![0; 4]),
            block(size, "revision", vec![]),
        ] {
            let mut client = Client {
                replies: VecDeque::from([block(size, "revision", prefix.clone()), last]),
                requests: vec![],
            };
            assert!(read(&mut client, json!({"id":"project","rel":"image.png"})).is_err());
            assert!(client.replies.is_empty());
        }
        for invalid in [
            block(MAX_FILE_BYTES + 1, "revision", vec![]),
            block(0, "", vec![]),
            block(1, "revision", vec![0; 2]),
        ] {
            let mut client = Client {
                replies: VecDeque::from([invalid]),
                requests: vec![],
            };
            assert!(read(&mut client, json!({"id":"project","rel":"image.png"})).is_err());
        }
        let mut client = Client {
            replies: VecDeque::from([block(0, "revision", vec![])]),
            requests: vec![],
        };
        assert!(read(&mut client, json!({"id":"project","rel":"empty"}))
            .unwrap()
            .is_empty());
    }
}
