use crate::atp::{AtpAddress, AtpRequestor, AtpResponder};
use anyhow::{Result, anyhow};
use tailtalk_packets::pap::{PapFunction, PapPacket};
use tokio::io::{AsyncRead, AsyncReadExt};

#[derive(Debug)]
pub struct PapClient {
    atp_requestor: AtpRequestor,
    atp_responder: AtpResponder,
    connection_id: u8,
    flow_quantum: u8,
    remote_addr: AtpAddress,
}

impl PapClient {
    pub fn new(atp_requestor: AtpRequestor, atp_responder: AtpResponder) -> Self {
        Self {
            atp_requestor,
            atp_responder,
            connection_id: 0,
            flow_quantum: 8,
            remote_addr: AtpAddress {
                network_number: 0,
                node_number: 0,
                socket_number: 0,
            },
        }
    }

    /// Open a PAP connection to the specified address
    pub async fn connect(&mut self, address: AtpAddress) -> Result<()> {
        self.remote_addr = address;

        // Send OpenConn request
        let open_packet = PapPacket {
            connection_id: self.atp_requestor.socket_number,
            function: PapFunction::OpenConn,
            sequence_num: 0,
            // OpenConn data: [Socket(1), FlowQuantum(1), WaitTime(2)]
            // We request flow quantum of 8.
            data: vec![self.atp_requestor.socket_number, 0x08, 0x00, 0x00],
        };

        let (user_bytes, data) = open_packet.to_atp_parts();

        tracing::info!("PAP: Sending OpenConn to {:?}", address);
        let (resp_data, resp_user_bytes) = self
            .atp_requestor
            .send_request(address, user_bytes, data.to_vec())
            .await?;

        // Parse response
        let reply = PapPacket::parse_from_atp(resp_user_bytes, &resp_data)?;

        if reply.function != PapFunction::OpenConnReply {
            return Err(anyhow!(
                "Unexpected response function: {:?}",
                reply.function
            ));
        }

        self.connection_id = reply.connection_id;

        if reply.data.len() >= 4 {
            let _server_socket = reply.data[0];
            self.flow_quantum = reply.data[1];
            let result = ((reply.data[2] as u16) << 8) | (reply.data[3] as u16);
            if result != 0 {
                return Err(anyhow!("PAP OpenConn failed with result code: {}", result));
            }
        }

        tracing::info!(
            "PAP connected! ID={}, Quantum={}",
            self.connection_id,
            self.flow_quantum
        );

        Ok(())
    }

    /// Send data (print job) to the connected printer
    pub async fn print(&mut self, data: &[u8]) -> Result<()> {
        self.print_stream(std::io::Cursor::new(data)).await
    }

    /// Send a print job from an async reader, pulling chunks on demand as the printer requests them.
    /// Each SendData cycle reads up to `flow_quantum * 574` bytes from the source in one shot.
    pub async fn print_stream<R: AsyncRead + Unpin>(&mut self, mut source: R) -> Result<()> {
        tracing::info!("PAP: Starting streaming print job");

        let mut eof = false;

        loop {
            let Some(req) = self.atp_responder.next().await else {
                return Err(anyhow!("ATP responder closed unexpectedly"));
            };

            let pap_req = PapPacket::parse_from_atp(req.user_bytes, &req.data)?;

            if pap_req.connection_id != self.connection_id {
                tracing::warn!(
                    "Ignored PAP packet with mismatched ID: {}",
                    pap_req.connection_id
                );
                continue;
            }

            match pap_req.function {
                PapFunction::SendData => {
                    let seq_num = pap_req.sequence_num;
                    tracing::debug!("PAP received SendData seq={}", seq_num);

                    let mut buf = vec![0u8; self.flow_quantum as usize * 574];
                    let n = source.read(&mut buf).await?;
                    buf.truncate(n);

                    if n == 0 {
                        eof = true;
                    }

                    let pap_resp = PapPacket {
                        connection_id: self.connection_id,
                        function: PapFunction::Data,
                        sequence_num: seq_num,
                        data: buf,
                    };
                    let (user_bytes, chunk_data) = pap_resp.to_atp_parts();
                    req.send_response(chunk_data.to_vec(), user_bytes).await?;

                    if let Some(rx) = req.release_rx {
                        tracing::debug!("PAP: Waiting for ATP Release");
                        let _ = rx.await;
                        tracing::debug!("PAP: Received ATP Release");
                    }

                    if eof {
                        break;
                    }
                }
                PapFunction::CloseConn => {
                    tracing::info!("PAP: Printer closed connection");
                    return Ok(());
                }
                _ => {}
            }
        }

        tracing::info!("PAP: Streaming print job finished, closing connection");
        self.close().await
    }

    pub async fn close(&mut self) -> Result<()> {
        let close_pkt = PapPacket {
            connection_id: self.connection_id,
            function: PapFunction::CloseConn,
            sequence_num: 0,
            data: vec![],
        };
        let (ub, d) = close_pkt.to_atp_parts();
        self.atp_requestor
            .send_request(self.remote_addr, ub, d.to_vec())
            .await?;
        Ok(())
    }

    pub async fn get_status(atp: AtpRequestor, address: AtpAddress) -> Result<String> {
        let pkt = PapPacket {
            connection_id: 0,
            function: PapFunction::SendStatus,
            sequence_num: 0,
            data: vec![],
        };
        let (ub, d) = pkt.to_atp_parts();
        let (resp_data, resp_ub) = atp.send_request(address, ub, d.to_vec()).await?;

        let reply = PapPacket::parse_from_atp(resp_ub, &resp_data)?;

        if reply.data.len() > 4 {
            Ok(String::from_utf8_lossy(&reply.data[4..]).to_string())
        } else {
            Ok("".to_string())
        }
    }
}
