use common::client_or_server::ClientOrServer;
use common::types::Types;
use req_resp::RequestOrResponse;
use server::conn::ServerConnData;
use server::conn::ServerStream;
use server::conn::ServerStreamData;
use server::conn::ServerToWriteMessage;
use server::stream_handler::ServerStreamHandlerHolder;

#[derive(Clone)]
pub(crate) struct ServerTypes;

impl Types for ServerTypes {
    type HttpStreamData = ServerStream;
    type HttpStreamSpecific = ServerStreamData;
    type ConnSpecific = ServerConnData;
    type StreamHandlerHolder = ServerStreamHandlerHolder;
    type ToWriteMessage = ServerToWriteMessage;

    const CLIENT_OR_SERVER: ClientOrServer = ClientOrServer::Server;
    const OUT_REQUEST_OR_RESPONSE: RequestOrResponse = RequestOrResponse::Response;
    const CONN_NDC: &'static str = "server conn";
}
