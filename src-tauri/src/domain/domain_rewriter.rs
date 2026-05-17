use crate::domain::Packet;
use bytes::Bytes;

#[derive(Debug, Clone)]
pub struct DomainRewriter {
    forward: Vec<(String, String)>,
    backward: Vec<(String, String)>,
}

impl DomainRewriter {
    pub fn new(upstream_server: &str) -> Self {
        let upstream_server = normalize_upstream_server(upstream_server);
        let upstream_hosts = upstream_mappings(&upstream_server);

        let mut forward = upstream_hosts.clone();
        let mut backward = upstream_hosts
            .iter()
            .map(|(local, upstream)| (upstream.clone(), local.clone()))
            .collect::<Vec<_>>();

        if upstream_server != "ppy.sh" {
            let official_hosts = upstream_mappings("ppy.sh");
            backward.extend(
                official_hosts
                    .iter()
                    .map(|(local, upstream)| (upstream.clone(), local.clone())),
            );
            forward.extend(official_hosts);
        }

        Self { forward, backward }
    }

    pub fn rewrite_forward(&self, body: Bytes) -> Bytes {
        self.rewrite(body, true)
    }

    pub fn rewrite_backward(&self, body: Bytes) -> Bytes {
        self.rewrite(body, false)
    }

    fn rewrite(&self, body: Bytes, is_forward: bool) -> Bytes {
        let mappings = if is_forward {
            &self.forward
        } else {
            &self.backward
        };

        let (mut packets, remaining) = Packet::parse_stream(&body);
        let mut changed = false;

        for packet in &mut packets {
            let mut payload_vec = packet.payload.clone();
            let mut packet_changed = false;

            for (from, to) in mappings {
                let from_bytes = from.as_bytes();
                let to_bytes = to.as_bytes();

                let mut pos = 0;
                while let Some(found_pos) = payload_vec[pos..]
                    .windows(from_bytes.len())
                    .position(|w| w == from_bytes)
                {
                    let absolute_pos = pos + found_pos;

                    self.try_patch_string_length(
                        &mut payload_vec,
                        absolute_pos,
                        from_bytes.len(),
                        to_bytes.len(),
                    );

                    payload_vec.splice(
                        absolute_pos..absolute_pos + from_bytes.len(),
                        to_bytes.iter().cloned(),
                    );

                    packet_changed = true;
                    pos = absolute_pos + to_bytes.len();
                }
            }

            if packet_changed {
                packet.payload = payload_vec;
                packet.header.length = packet.payload.len() as u32;
                changed = true;
            }
        }

        if !changed {
            return body;
        }

        let mut out = Vec::with_capacity(body.len() + 32);
        for p in packets {
            out.extend_from_slice(&p.to_bytes());
        }
        out.extend(remaining);
        Bytes::from(out)
    }

    fn try_patch_string_length(
        &self,
        payload: &mut [u8],
        data_pos: usize,
        old_len: usize,
        new_len: usize,
    ) {
        for i in 1..256 {
            if data_pos < i {
                break;
            }

            if payload[data_pos - i] == 0x0b {
                let len_idx = data_pos - i + 1;

                if len_idx < payload.len() {
                    let current_str_len = payload[len_idx] as i32;

                    if current_str_len >= (i as i32 - 1) {
                        let new_val = (current_str_len - (old_len as i32) + (new_len as i32)) as u8;
                        payload[len_idx] = new_val;
                        break;
                    }
                }
            }
        }
    }
}

fn normalize_upstream_server(upstream_server: &str) -> String {
    upstream_server
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_matches('/')
        .to_lowercase()
}

fn upstream_mappings(upstream_server: &str) -> Vec<(String, String)> {
    [
        ("osu.localhost", format!("osu.{}", upstream_server)),
        ("c.localhost", format!("c.{}", upstream_server)),
        ("a.localhost", format!("a.{}", upstream_server)),
        ("b.localhost", format!("b.{}", upstream_server)),
        ("i.localhost", format!("i.{}", upstream_server)),
    ]
    .into_iter()
    .map(|(local, upstream)| (local.to_string(), upstream))
    .collect()
}
