use std::any::Any;
use std::fs;
use std::io;
use std::path::Path;

use tokio_core::reactor;
#[cfg(unix)]
use tokio_uds::UnixListener;
#[cfg(unix)]
use tokio_uds::UnixStream;

use futures::future::err;
use futures::future::ok;
use futures::stream::Stream;
use futures::Future;

use socket::AnySocketAddr;
use socket::StreamItem;
use socket::ToClientStream;
use socket::ToServerStream;
use socket::ToSocketListener;
use socket::ToTokioListener;
use std::fmt;
use std::path::PathBuf;
use ServerConf;

#[derive(Debug, Clone)]
pub struct SocketAddrUnix(pub(crate) PathBuf);

impl From<PathBuf> for SocketAddrUnix {
    fn from(p: PathBuf) -> Self {
        SocketAddrUnix(p)
    }
}

impl From<&Path> for SocketAddrUnix {
    fn from(p: &Path) -> Self {
        SocketAddrUnix(p.into())
    }
}

impl From<&str> for SocketAddrUnix {
    fn from(p: &str) -> Self {
        SocketAddrUnix(p.into())
    }
}

impl From<String> for SocketAddrUnix {
    fn from(p: String) -> Self {
        SocketAddrUnix(p.into())
    }
}

impl fmt::Display for SocketAddrUnix {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0.display(), f)
    }
}

impl ToSocketListener for SocketAddrUnix {
    #[cfg(unix)]
    fn to_listener(&self, _conf: &ServerConf) -> io::Result<Box<ToTokioListener + Send>> {
        debug!("binding socket to {}", self);
        Ok(Box::new(::std::os::unix::net::UnixListener::bind(&self.0)?))
    }

    #[cfg(not(unix))]
    fn to_listener(&self, _conf: &ServerConf) -> io::Result<Box<ToTokioListener + Send>> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "cannot use unix sockets on non-unix",
        ))
    }

    fn cleanup(&self) {
        if self.0.exists() {
            fs::remove_file(&self.0).expect("removing socket during shutdown");
        }
    }
}

#[cfg(unix)]
impl ToTokioListener for ::std::os::unix::net::UnixListener {
    fn to_tokio_listener(self: Box<Self>, handle: &reactor::Handle) -> Box<ToServerStream> {
        Box::new(UnixListener::from_listener(*self, handle).unwrap())
    }

    fn local_addr(&self) -> io::Result<AnySocketAddr> {
        let addr = self.local_addr().unwrap();
        let path = addr.as_pathname().unwrap();
        Ok(AnySocketAddr::Unix(SocketAddrUnix::from(path)))
    }
}

#[cfg(unix)]
impl ToServerStream for UnixListener {
    fn incoming(
        self: Box<Self>,
    ) -> Box<Stream<Item = (Box<StreamItem>, Box<Any>), Error = io::Error>> {
        let stream = (*self).incoming().map(|(stream, addr)| {
            (
                Box::new(stream) as Box<StreamItem>,
                Box::new(addr) as Box<Any>,
            )
        });
        Box::new(stream)
    }
}

impl ToClientStream for SocketAddrUnix {
    #[cfg(unix)]
    fn connect(
        &self,
        handle: &reactor::Handle,
    ) -> Box<Future<Item = Box<StreamItem>, Error = io::Error> + Send> {
        let stream = UnixStream::connect(&self.0, &handle);
        if stream.is_ok() {
            Box::new(ok(Box::new(stream.unwrap()) as Box<StreamItem>))
        } else {
            Box::new(err(stream.unwrap_err()))
        }
    }

    #[cfg(not(unix))]
    fn connect(
        &self,
        _handle: &reactor::Handle,
    ) -> Box<Future<Item = Box<StreamItem>, Error = io::Error> + Send> {
        use futures::future;
        Box::new(future::err(io::Error::new(
            io::ErrorKind::Other,
            "cannot use unix sockets on non-unix",
        )))
    }
}

#[cfg(unix)]
impl StreamItem for UnixStream {
    fn is_tcp(&self) -> bool {
        false
    }

    fn set_nodelay(&self, _no_delay: bool) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Cannot set nodelay on unix domain socket",
        ))
    }
}
