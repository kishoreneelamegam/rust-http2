use bytes::Bytes;
use error;
use result;
use ErrorCode;
use Headers;

/// Synchronous callback of incoming data
pub(crate) trait StreamHandlerInternal: 'static {
    /// DATA frame received
    fn data_frame(&mut self, data: Bytes, end_stream: bool) -> result::Result<()>;
    /// Trailers HEADERS received
    fn trailers(&mut self, trailers: Headers) -> result::Result<()>;
    /// RST_STREAM frame received
    fn rst(self, error_code: ErrorCode) -> result::Result<()>;
    /// Any other error
    fn error(self, error: error::Error) -> result::Result<()>;
}
