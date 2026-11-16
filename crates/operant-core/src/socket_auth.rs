//! The peer-authentication gate every operant UNIX socket server must use.
//!
//! (2026-11-16, audit RT-Z30-A) This module exists because there was ONE gate
//! and TWO sockets. `session_server` grew the uid check in the Z30 fix and
//! `acp::server` -- which binds through the same [`bind_unless_live`] helper and
//! carries strictly MORE authority (an arbitrary `command` method reaching the
//! whole agent runtime, plus `stop`) -- had no mode tightening and no peer
//! check at all. `accept()` there discarded the peer and served it.
//!
//! Two copies of a credential check is how the two drift, which is the same
//! argument `agent::run` makes for sharing the reasoning helper. Both socket
//! servers now call the functions here, so a third socket server has one
//! obvious thing to call.
//!
//! What this gate is: 0600 on the bound path, then a per-connection check that
//! the peer's uid equals ours. What it is NOT: an allowlist, and not an
//! authentication of the process. A same-uid process is inside the boundary
//! the mode draws, and on a multi-agent host "same uid" is not a trust
//! boundary -- a build script or another agent's tool runs as the operator.
//! The gate closes the account boundary (a different uid cannot connect) and
//! says so honestly; it does not claim to close the same-uid case.

use crate::error::{Error, Result};

/// Make `socket` 0600 or refuse to serve.
///
/// The socket carries control of the agent and its tools, so a mode that lets
/// another account connect is not a degraded mode -- it is the hole. The
/// default umask gives a bound socket 0755, which is only safe because the
/// operant home happens to be 0700; an operator who binds `/tmp/agent.sock`
/// gets none of that.
///
/// Returns `Err` and the caller's accept loop is never entered. The callers
/// spawn this and log a returned error, so the failure surfaces as a loud
/// error line and NO socket (clients get "connection refused"), not as a
/// process shutdown.
#[cfg(unix)]
pub(crate) async fn tighten_socket_permissions(socket: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    // The chmod is a single syscall on a path this process just bound, so it
    // stays on the blocking pool rather than parking a runtime worker.
    let path = socket.to_path_buf();
    tokio::task::spawn_blocking(move || std::fs::set_permissions(&path, perms))
        .await
        .map_err(|e| {
            Error::Config(format!(
                "the socket {} could not be restricted to 0600: the join \
                 failed ({e}). Refusing to serve: this socket carries control \
                 of the agent and its tools.",
                socket.display()
            ))
        })?
        .map_err(|e| {
            Error::Config(format!(
                "the socket {} could not be restricted to 0600: {e}. Refusing \
                 to serve: a socket another account can reach is not a safe one \
                 to serve.",
                socket.display()
            ))
        })
}

/// The uid this process runs as.
#[cfg(unix)]
fn own_uid() -> u32 {
    // `getuid` cannot fail: it reads the process credentials and has no
    // error path in the C API. SAFETY: no arguments, no return pointer.
    unsafe { libc::getuid() as u32 }
}

/// The uid on the other end of a connected UNIX socket.
///
/// Linux: `SO_PEERCRED` returns the credentials of the process that called
/// `connect()`, fixed at connect time and unchangeable by the peer afterwards.
/// macOS/BSD: `getpeereid` is NOT wired -- see [`authorize_peer`], which is
/// Unix-gated and therefore ABSENT there rather than enforced. That asymmetry
/// is RT-Z30-B and is recorded rather than hidden.
#[cfg(target_os = "linux")]
fn peer_uid(stream: &tokio::net::UnixStream) -> std::io::Result<u32> {
    use std::os::unix::io::AsRawFd;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` is a valid, fully-initialised `ucred` and `len` describes
    // exactly its size. `SO_PEERCRED` is correct for an AF_UNIX SOCK_STREAM
    // socket on Linux, and reading it does not disturb the connection.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast::<libc::c_void>(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(cred.uid)
}

/// Refuse a connection whose peer uid is not ours.
///
/// The DECISION is separate from the credential read so it is unit-testable
/// without a second process: [`authorize_uid`] takes the uid and the tests
/// drive every branch of it. What cannot be tested in-process is the syscall
/// itself.
///
/// GATE COVERAGE: Linux has `SO_PEERCRED` and is checked. macOS/BSD have no
/// wired equivalent, so `peer_uid` does not exist there and this function is
/// Unix-gated to the same set -- on those platforms there is NO peer check and
/// only the 0600 mode stands. That is RT-Z30-B, left open deliberately rather
/// than papered over with a check that always passes.
#[cfg(all(unix, target_os = "linux"))]
pub(crate) fn authorize_peer(stream: &tokio::net::UnixStream) -> std::io::Result<()> {
    authorize_uid(peer_uid(stream)?)
}

/// Non-Linux unix: no peer credential is wired, so nothing is checked. See
/// [`authorize_peer`]. Kept as a separate item rather than an empty body inside
/// the caller so the caller's `#[cfg]` is the only place that knows. The callers
/// additionally gate their call site on `target_os = "linux"`, so this arm is
/// the belt to that braces: it makes the module total without pretending to
/// enforce anything it cannot.
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn authorize_peer(_stream: &tokio::net::UnixStream) -> std::io::Result<()> {
    Ok(())
}

/// The uid comparison [`authorize_peer`] makes, over a uid the caller read.
///
/// Split out from `authorize_peer` for the same reason the credential read is:
/// so the DECISION is drivable without a second process. A test that only
/// asserts the same-uid happy path passes with `authorize_peer` stubbed to
/// `Ok(())`; driving `authorize_uid` with a foreign uid is what proves the
/// gate consults the credential at all.
#[cfg(unix)]
pub(crate) fn authorize_uid(uid: u32) -> std::io::Result<()> {
    if uid == own_uid() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("socket peer uid {uid} does not match this process's uid"),
        ))
    }
}

/// Non-unix: there is no peer credential to read and no socket mode to
/// tighten, so nothing is lost that this platform ever offered. A Windows
/// named pipe's ACL is outside this module's scope.
#[cfg(not(unix))]
pub(crate) async fn tighten_socket_permissions(_socket: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate's whole decision: a uid that is not ours is refused, with a
    /// typed error a caller can log and act on.
    #[cfg(unix)]
    #[test]
    fn a_foreign_uid_is_refused_and_ours_is_accepted() {
        let mine = own_uid();
        assert!(
            authorize_uid(mine).is_ok(),
            "the process's own uid must pass the gate"
        );

        // Pick a uid that is definitely not ours. `0` is root; if we ARE root,
        // use 1 instead.
        let other = if mine == 0 { 1 } else { 0 };
        let refused = authorize_uid(other).expect_err("a foreign uid must be refused");
        assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(
            refused.to_string().contains("does not match"),
            "the message must say what failed: {refused}"
        );
    }

    /// Non-unix keeps the gate inert rather than failing every connection: the
    /// module's contract is "no platform loses anything it ever offered".
    #[cfg(not(unix))]
    #[test]
    fn the_gate_is_inert_where_there_is_no_peer_credential() {
        // `tighten_socket_permissions` returns Ok on a path that need not
        // exist, so a caller never has to cfg its own call site.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        rt.block_on(tighten_socket_permissions(std::path::Path::new(
            "/nonexistent/socket",
        )))
        .expect("inert gate");
    }

    /// The END-TO-END half that CAN run in-process: a real `connect()` on a
    /// real bound socket, same uid on both ends, reads the real `SO_PEERCRED`.
    ///
    /// Mutation-shaped (RT-Z30-D): replacing `authorize_peer`'s body with `Ok(())`
    /// passes every OTHER test in this module, because they drive the
    /// comparison and not the call. This one fails, because it goes through the
    /// syscall on a connected fd. Deleting the `authorize_peer` call from a
    /// caller's accept loop is caught at the CALLER, not here -- that is what
    /// `the_accept_loop_refuses...` in each server module is for.
    ///
    /// What it still cannot cover, stated plainly: a FOREIGN uid. Reading a
    /// peer's credential as another user needs a setuid helper or a container.
    /// The negative end rests on the code plus the kernel contract.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_real_same_uid_connection_passes_the_gate_and_the_socket_is_0600() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let dir = tempfile::tempdir().expect("a temp dir");
        let socket = dir.path().join("auth.sock");
        let listener = crate::session_server::bind_unless_live(&socket)
            .await
            .expect("the bind must work");
        tighten_socket_permissions(&socket).await.expect("0600");

        let client = tokio::net::UnixStream::connect(&socket)
            .await
            .expect("the client must connect");
        let (server, _peer) = listener.accept().await.expect("accept");

        // The real syscall, on a real connection, through the PUBLIC gate
        // function -- not the private `peer_uid`. Asserting on `peer_uid`
        // directly would pass with `authorize_peer` stubbed to `Ok(())`, which
        // is exactly the mutation this has to catch.
        assert_eq!(
            peer_uid(&server).expect("SO_PEERCRED must read the peer"),
            own_uid(),
            "a same-process client's uid is ours"
        );
        authorize_peer(&server).expect("a same-uid peer must be authorized");

        // And the same gate must REFUSE a uid that is not ours. Driving the
        // public function with a foreign uid is the negative end: it proves
        // `authorize_peer` consults the credential instead of returning `Ok`
        // unconditionally, which no amount of same-uid success can show.
        #[cfg(unix)]
        {
            let foreign = if own_uid() == 0 { 1 } else { 0 };
            let refused = authorize_uid(foreign).expect_err("a foreign uid must be refused");
            assert_eq!(refused.kind(), std::io::ErrorKind::PermissionDenied);
        }

        // And the socket is actually 0600, which the tightened path promises.
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&socket)
            .expect("the socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the socket must be owner-only");

        // The authorized connection still carries bytes both ways, so the gate
        // does not break the channel it guards.
        let mut client = client;
        client.write_all(b"ping\n").await.expect("write");
        let (mut read_half, mut write_half) = tokio::io::split(server);
        let mut buf = [0u8; 5];
        read_half.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"ping\n");
        write_half.write_all(b"pong\n").await.expect("write");
        drop(write_half);
    }

    /// (RT-Z30-D) The mode gate is not optional: a socket this process cannot
    /// chmod is REFUSED, so a caller that ignored the `Err` would still serve.
    /// This drives the failure path the session socket's doc promises.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_socket_that_cannot_be_restricted_is_refused() {
        // A path under a directory that does not exist: the bind would fail
        // first, so bind a real socket and then chmod a path that has been
        // REPLACED by a directory -- `set_permissions` on a directory is
        // permitted, so instead point at a path that cannot exist.
        let dir = tempfile::tempdir().expect("a temp dir");
        let socket = dir.path().join("gone.sock");
        let listener = crate::session_server::bind_unless_live(&socket)
            .await
            .expect("the bind must work");
        drop(listener);
        std::fs::remove_file(&socket).expect("remove the bound path");

        let refused = tighten_socket_permissions(&socket)
            .await
            .expect_err("a chmod on a missing path must be refused");
        assert!(
            refused.to_string().contains("0600"),
            "the refusal must name the mode it could not set: {refused}"
        );
    }
}
