//! Tests for server.

extern crate bytes;
#[macro_use]
extern crate log;
extern crate futures;
extern crate futures_cpupool;
extern crate httpbis;
extern crate regex;
extern crate tokio_core;
extern crate tokio_tls_api;

extern crate httpbis_test;
use httpbis_test::*;

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::Bytes;

use tokio_core::reactor;

use std::io::Read as _Read;
use std::io::Write as _Write;
use std::thread;

use futures::future::Future;
use futures::stream;
use futures::stream::Stream;
use futures::sync::oneshot;
use futures::Async;
use futures::Poll;

use httpbis::for_test::solicit::frame::headers::*;
use httpbis::for_test::solicit::frame::settings::HttpSetting;
use httpbis::for_test::solicit::frame::settings::SettingsFrame;
use httpbis::for_test::solicit::DEFAULT_SETTINGS;
use httpbis::*;

use std::iter::FromIterator;
use std::net::TcpStream;
use std::sync::mpsc;

#[cfg(unix)]
extern crate tempdir;
#[cfg(unix)]
extern crate unix_socket;
use futures_cpupool::CpuPool;
#[cfg(unix)]
use unix_socket::UnixStream;

#[test]
fn simple_new() {
    init_logger();

    let server = ServerOneConn::new_fn(0, |_, req, mut resp| {
        resp.send_headers(Headers::ok_200())?;
        resp.pull_from_stream(req.make_stream())?;
        Ok(())
    });

    let mut tester = HttpConnTester::connect(server.port());
    tester.send_preface();
    tester.settings_xchg();

    let mut headers = Headers::new();
    headers.add(":method", "GET");
    headers.add(":path", "/aabb");
    headers.add(":scheme", "http");
    tester.send_headers(1, headers, false);

    tester.send_data(1, b"abcd", true);

    let recv_headers = tester.recv_frame_headers_check(1, false);
    assert_eq!("200", recv_headers.get(":status"));

    assert_eq!(&b"abcd"[..], &tester.recv_frame_data_check(1, true)[..]);

    assert_eq!(0, server.dump_state().streams.len());
}

#[test]
fn custom_drop_callback() {
    init_logger();

    let server = ServerOneConn::new_fn(0, |_, _req, mut resp| {
        resp.set_drop_callback(|resp| Ok(resp.send_internal_error_500("test test")?));
        Err(httpbis::Error::User("my".to_owned()))
    });

    let mut tester = HttpConnTester::connect(server.port());
    tester.send_preface();
    tester.settings_xchg();

    let mut headers = Headers::new();
    headers.add(":method", "GET");
    headers.add(":path", "/aabb");
    headers.add(":scheme", "http");
    tester.send_headers(1, headers, false);

    tester.send_data(1, b"abcd", true);

    let recv_headers = tester.recv_frame_headers_check(1, false);
    assert_eq!("500", recv_headers.get(":status"));
}

#[test]
fn panic_in_handler() {
    init_logger();

    let server = ServerOneConn::new_fn(0, |_, req, mut resp| {
        if req.headers.path() == "/panic" {
            panic!("requested");
        } else {
            resp.send_found_200_plain_text("hi there")?;
            Ok(())
        }
    });

    let mut tester = HttpConnTester::connect(server.port());
    tester.send_preface();
    tester.settings_xchg();

    info!("test /hello");

    {
        let resp = tester.get(1, "/hello");
        assert_eq!(200, resp.headers.status());
        assert_eq!(&b"hi there"[..], &resp.body[..]);
    }

    info!("test /panic");

    {
        tester.send_get(3, "/panic");
        tester.recv_rst_frame_check(3, ErrorCode::InternalError);
    }

    info!("test /world");

    {
        let resp = tester.get(5, "/world");
        assert_eq!(200, resp.headers.status());
        assert_eq!(&b"hi there"[..], &resp.body[..]);
    }

    assert_eq!(0, server.dump_state().streams.len());
}

#[test]
fn panic_in_stream() {
    init_logger();

    let server = ServerOneConn::new_fn(0, |_, req, mut resp| {
        if req.headers.path() == "/panic" {
            let stream = HttpStreamAfterHeaders::new(stream::iter_ok((0..2).map(|_| {
                panic!("should reset stream");
            })));
            resp.send_headers(Headers::ok_200())?;
            resp.pull_from_stream(stream)?;
        } else {
            resp.send_found_200_plain_text("hi there")?;
        }
        Ok(())
    });

    let mut tester = HttpConnTester::connect(server.port());
    tester.send_preface();
    tester.settings_xchg();

    info!("test /hello");

    {
        let resp = tester.get(1, "/hello");
        assert_eq!(200, resp.headers.status());
        assert_eq!(&b"hi there"[..], &resp.body[..]);
    }

    info!("test /panic");

    {
        tester.send_get(3, "/panic");
        tester.recv_frame_headers_check(3, false);
        tester.recv_rst_frame();
    }

    info!("test /world");

    {
        let resp = tester.get(5, "/world");
        assert_eq!(200, resp.headers.status());
        assert_eq!(&b"hi there"[..], &resp.body[..]);
    }

    assert_eq!(0, server.dump_state().streams.len());
}

#[test]
fn response_large() {
    init_logger();

    let mut large_resp = Vec::new();
    while large_resp.len() < 100_000 {
        if large_resp.len() != 0 {
            write!(&mut large_resp, ",").unwrap();
        }
        let len = large_resp.len();
        write!(&mut large_resp, "{}", len).unwrap();
    }

    let large_resp_copy = large_resp.clone();

    let server = ServerOneConn::new_fn(0, move |_, _req, mut resp| {
        resp.send_message(SimpleHttpMessage {
            headers: Headers::ok_200(),
            body: Bytes::from(large_resp_copy.clone()),
        })?;
        Ok(())
    });

    // TODO: rewrite with TCP
    let client = Client::new_plain(BIND_HOST, server.port(), Default::default()).expect("connect");
    let resp = client
        .start_post("/foobar", "localhost", Bytes::from(&b""[..]))
        .collect()
        .wait()
        .expect("wait");
    assert_eq!(large_resp.len(), resp.body.len());
    assert_eq!(
        (large_resp.len(), &large_resp[..]),
        (resp.body.len(), &resp.body[..])
    );

    assert_eq!(0, server.dump_state().streams.len());
}

#[test]
fn rst_stream_on_data_without_stream() {
    init_logger();

    let server = ServerTest::new();

    let mut tester = HttpConnTester::connect(server.port);
    tester.send_preface();
    tester.settings_xchg();

    // DATA frame without open stream
    tester.send_data(11, &[10, 20, 30], false);

    tester.recv_goaway_frame_check(ErrorCode::StreamClosed);

    tester.recv_eof();
}

#[test]
fn exceed_max_frame_size() {
    init_logger();

    let server = ServerTest::new();

    let mut tester = HttpConnTester::connect(server.port);
    tester.send_preface();
    tester.settings_xchg();

    tester.send_data(1, &[0; 17_000], false);

    tester.recv_eof();

    let mut tester = HttpConnTester::connect(server.port);
    tester.send_preface();
    tester.settings_xchg();

    assert_eq!(200, tester.get(1, "/echo").headers.status());
}

#[test]
fn increase_frame_size() {
    init_logger();

    let server = ServerTest::new();

    let mut tester = HttpConnTester::connect(server.port);
    tester.send_preface();
    tester.settings_xchg();

    let mut frame = SettingsFrame::new();
    frame.settings.push(HttpSetting::MaxFrameSize(20000));
    tester.send_recv_settings(frame);

    tester.send_get(1, "/blocks/30000/1");
    assert_eq!(200, tester.recv_frame_headers_check(1, false).status());
    assert_eq!(20000, tester.recv_frame_data_check(1, false).len());
    assert_eq!(10000, tester.recv_frame_data_tail(1).len());
}

#[test]
fn exceed_window_size() {
    init_logger();

    let server = ServerTest::new();

    let mut tester = HttpConnTester::connect(server.port);
    tester.send_preface();
    tester.settings_xchg();

    let mut frame = SettingsFrame::new();
    frame.settings.push(HttpSetting::MaxFrameSize(
        tester.peer_settings.initial_window_size + 5,
    ));
    tester.send_recv_settings(frame);

    let data = Vec::from_iter((0..tester.peer_settings.initial_window_size + 3).map(|_| 2));

    // Deliberately set wrong out_windows_size so `send_data` wouldn't fail.
    tester.out_window_size.0 += 10000000;
    tester.send_data(1, &data, false);
    tester.recv_eof();

    let mut tester = HttpConnTester::connect(server.port);
    tester.send_preface();
    tester.settings_xchg();

    assert_eq!(200, tester.get(1, "/echo").headers.status());
}

#[test]
fn stream_window_gt_conn_window() {
    init_logger();

    let server = ServerTest::new();

    let mut tester = HttpConnTester::connect(server.port);
    tester.send_preface();
    tester.settings_xchg();

    let w = DEFAULT_SETTINGS.initial_window_size;

    // May need to be changed if server defaults are changed
    assert_eq!(w as i32, tester.in_window_size.size());

    tester.send_recv_settings(SettingsFrame::from_settings(vec![
        HttpSetting::InitialWindowSize(w * 2),
        HttpSetting::MaxFrameSize(w * 10),
    ]));

    // Now new stream window is gt than conn window

    let w = tester.peer_settings.initial_window_size;
    tester.send_get(1, &format!("/blocks/{}/{}", w, 2));

    assert_eq!(200, tester.recv_frame_headers_check(1, false).status());
    assert_eq!(w as usize, tester.recv_frame_data_check(1, false).len());

    let server_sn = server.server.dump_state().wait().expect("state");
    assert_eq!(0, server_sn.single_conn().1.out_window_size);
    assert_eq!(
        w as i32,
        server_sn.single_conn().1.single_stream().1.out_window_size
    );
    assert_eq!(
        w as isize,
        server_sn
            .single_conn()
            .1
            .single_stream()
            .1
            .pump_out_window_size
    );
    assert_eq!(
        0,
        server_sn
            .single_conn()
            .1
            .single_stream()
            .1
            .queued_out_data_size
    );

    tester.send_window_update_conn(w + 1);
    tester.send_window_update_stream(1, w + 1);

    assert_eq!(w as usize, tester.recv_frame_data_tail(1).len());
}

#[test]
fn do_not_poll_when_not_enough_window() {
    init_logger();

    let polls = Arc::new(AtomicUsize::new(0));
    let polls_copy = polls.clone();

    let server = ServerOneConn::new_fn(0, move |_, _, mut resp| {
        struct StreamImpl {
            polls: Arc<AtomicUsize>,
        }

        impl Stream for StreamImpl {
            type Item = Bytes;
            type Error = Error;

            fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
                let polls = self.polls.fetch_add(1, Ordering::SeqCst);
                Ok(Async::Ready(match polls {
                    0 | 1 | 2 => Some(Bytes::from(vec![
                        polls as u8;
                        DEFAULT_SETTINGS.initial_window_size
                            as usize
                    ])),
                    _ => None,
                }))
            }
        }

        resp.send_headers(Headers::ok_200())?;
        resp.pull_bytes_from_stream(StreamImpl {
            polls: polls_copy.clone(),
        })?;

        Ok(())
    });

    let mut tester = HttpConnTester::connect(server.port());
    tester.send_preface();
    tester.settings_xchg();

    tester.send_recv_settings(SettingsFrame::from_settings(vec![
        HttpSetting::MaxFrameSize(DEFAULT_SETTINGS.initial_window_size * 5),
    ]));

    tester.send_get(1, "/fgfg");
    assert_eq!(200, tester.recv_frame_headers_check(1, false).status());
    assert_eq!(
        DEFAULT_SETTINGS.initial_window_size as usize,
        tester.recv_frame_data_check(1, false).len()
    );

    assert_eq!(1, polls.load(Ordering::SeqCst));

    tester.send_window_update_conn(DEFAULT_SETTINGS.initial_window_size);
    tester.send_window_update_stream(1, DEFAULT_SETTINGS.initial_window_size);

    assert_eq!(
        DEFAULT_SETTINGS.initial_window_size as usize,
        tester.recv_frame_data_check(1, false).len()
    );

    assert_eq!(2, polls.load(Ordering::SeqCst));
}

#[test]
pub fn server_sends_continuation_frame() {
    init_logger();

    let mut headers = Headers::ok_200();
    for i in 0..1000 {
        headers.add(
            format!("abcdefghijklmnop{}", i),
            format!("ABCDEFGHIJKLMNOP{}", i),
        );
    }

    let headers_copy = headers.clone();

    let server = ServerOneConn::new_fn(0, move |_, _req, mut resp| {
        resp.send_message(SimpleHttpMessage {
            headers: headers_copy.clone(),
            body: Bytes::from("there"),
        })?;
        Ok(())
    });

    let mut tester = HttpConnTester::connect(server.port());
    tester.send_preface();
    tester.settings_xchg();

    tester.send_get(1, "/long-header-list");
    let (headers_frame, headers_recv, cont_count) = tester.recv_frame_headers_decode();
    assert!(headers_frame.flags.is_set(HeadersFlag::EndHeaders));
    assert!(cont_count > 0);
    assert_eq!(headers, headers_recv);

    assert_eq!(&b"there"[..], &tester.recv_frame_data_tail(1)[..]);
}

#[test]
pub fn http_1_1() {
    init_logger();

    let server = ServerTest::new();

    let mut tcp_stream = TcpStream::connect((BIND_HOST, server.port)).expect("connect");

    tcp_stream.write_all(b"GET / HTTP/1.1\n").expect("write");

    let mut read = Vec::new();
    tcp_stream.read_to_end(&mut read).expect("read");
    assert!(
        &read.starts_with(b"HTTP/1.1 500 Internal Server Error\r\n"),
        "{:?}",
        BsDebug(&read)
    );
}

#[cfg(unix)]
#[test]
pub fn http_1_1_unix() {
    init_logger();

    let tempdir = tempdir::TempDir::new("rust_http2_test").unwrap();
    let socket_path = tempdir.path().join("test_socket");
    let _server = ServerTest::new_unix(socket_path.to_str().unwrap().to_owned());

    let mut unix_stream = UnixStream::connect(socket_path).expect("connect");

    unix_stream.write_all(b"GET / HTTP/1.1\n").expect("write");

    let mut read = Vec::new();
    unix_stream.read_to_end(&mut read).expect("read");
    assert!(
        &read.starts_with(b"HTTP/1.1 500 Internal Server Error\r\n"),
        "{:?}",
        BsDebug(&read)
    );
}

#[test]
fn external_event_loop() {
    init_logger();

    let (tx, rx) = mpsc::channel();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let t = thread::spawn(move || {
        let mut core = reactor::Core::new().expect("Core::new");

        let mut servers = Vec::new();
        for _ in 0..2 {
            let mut server = ServerBuilder::new_plain();
            server.event_loop = Some(core.remote());
            server.set_port(0);
            server.service.set_service_fn("/", |_, _, mut resp| {
                resp.send_found_200_plain_text("aabb")?;
                Ok(())
            });
            servers.push(server.build().expect("server"));
        }

        tx.send(
            servers
                .iter()
                .map(|s| s.local_addr().port().unwrap())
                .collect::<Vec<_>>(),
        )
        .expect("send");

        core.run(shutdown_rx).expect("run");
    });

    let ports = rx.recv().expect("recv");

    for port in ports {
        let client = Client::new_plain(BIND_HOST, port, ClientConf::new()).expect("client");
        let resp = client
            .start_get("/", "localhost")
            .collect()
            .wait()
            .expect("ok");
        assert_eq!(b"aabb", &resp.body[..]);
    }

    shutdown_tx.send(()).expect("send");

    t.join().expect("thread join");
}

/// Example of moving heavy computation to CPU tool
#[test]
fn example_cpu_pool() {
    init_logger();

    let cpu_pool = CpuPool::new(2);

    let mut server = ServerBuilder::new_plain();
    server
        .service
        .set_service_fn("/foo", move |_, _, mut resp| {
            cpu_pool
                .spawn_fn(move || {
                    if let Err(e) = resp.send_found_200_plain_text("hello") {
                        warn!("failed to send response: {:?}", e);
                    }
                    Ok::<_, ()>(())
                })
                .forget();
            Ok(())
        });
    server.set_port(0);
    let server = server.build().expect("server");

    let client = Client::new_plain(
        "127.0.0.1",
        server.local_addr().port().unwrap(),
        Default::default(),
    )
    .expect("client");

    let response = client
        .start_get("/foo", "localhost")
        .collect()
        .wait()
        .expect("get");
    assert_eq!(200, response.headers.status());
    assert_eq!(b"hello", response.body.as_ref());
}
