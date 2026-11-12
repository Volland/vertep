//! Throwaway containers for tests against real services: etcd and a lightweight Kubernetes
//! (k3s, API server only). Tests using them run only when `VERTEP_CONTAINER_TESTS` is set:
//! `1` runs them when Docker answers and skips them otherwise, `require` fails without Docker.
//! `VERTEP_DOCKER` names another Docker-compatible CLI (for example `podman`). Every container
//! is labelled `vertep-test` and removed when its handle drops.
#![allow(dead_code)]

use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// The etcd image, pinned.
pub const ETCD_IMAGE: &str = "quay.io/coreos/etcd:v3.5.17";
/// The k3s image, pinned.
pub const K3S_IMAGE: &str = "rancher/k3s:v1.31.4-k3s1";

/// The container CLI.
pub fn docker() -> String {
    std::env::var("VERTEP_DOCKER").unwrap_or_else(|_| "docker".into())
}

fn run(args: &[&str]) -> Output {
    Command::new(docker())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{} {args:?}: {e}", docker()))
}

fn run_ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "{} {args:?} failed: {}",
        docker(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Whether container tests run: `false` (with a `skipped:` message) unless
/// `VERTEP_CONTAINER_TESTS` asks for them and Docker answers.
pub fn enabled(what: &str) -> bool {
    let mode = std::env::var("VERTEP_CONTAINER_TESTS").unwrap_or_default();
    if mode.is_empty() || mode == "0" {
        eprintln!("skipped: {what} runs against containers; set VERTEP_CONTAINER_TESTS=1");
        return false;
    }
    let up = Command::new(docker())
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !up {
        assert!(mode != "require", "{} is not available", docker());
        eprintln!("skipped: {what}: {} is not available", docker());
    }
    up
}

static SEQ: AtomicU32 = AtomicU32::new(0);

/// A unique, DNS-label-safe name with `tag`.
pub fn unique(tag: &str) -> String {
    format!(
        "{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    )
}

/// A running container, removed on drop.
#[derive(Debug)]
pub struct Container {
    pub name: String,
}

impl Container {
    /// Starts `image` detached with `docker_args` and the command `args`.
    pub fn start(tag: &str, image: &str, docker_args: &[&str], args: &[&str]) -> Container {
        let name = unique(tag);
        let mut all = vec![
            "run",
            "-d",
            "--rm",
            "--label",
            "vertep-test",
            "--name",
            &name,
        ];
        all.extend_from_slice(docker_args);
        all.push(image);
        all.extend_from_slice(args);
        run_ok(&all);
        Container { name }
    }

    /// Creates and starts `image` detached, kept after it exits so it can be started again
    /// (removed on drop all the same).
    pub fn start_kept(tag: &str, image: &str, docker_args: &[&str], args: &[&str]) -> Container {
        let name = unique(tag);
        let mut all = vec!["run", "-d", "--label", "vertep-test", "--name", &name];
        all.extend_from_slice(docker_args);
        all.push(image);
        all.extend_from_slice(args);
        run_ok(&all);
        Container { name }
    }

    /// Runs `docker <verb> <args...> <name>`; whether it succeeded.
    pub fn docker(&self, verb: &[&str]) -> bool {
        let mut all = verb.to_vec();
        all.push(&self.name);
        run(&all).status.success()
    }

    /// The host port `port/tcp` is published on.
    pub fn host_port(&self, port: u16) -> u16 {
        let out = run_ok(&["port", &self.name, &format!("{port}/tcp")]);
        out.lines()
            .find_map(|l| l.rsplit(':').next()?.trim().parse().ok())
            .unwrap_or_else(|| panic!("no host port for {port} in {out:?}"))
    }

    /// Runs `cmd` in the container; its stdout when it succeeds.
    pub fn exec(&self, cmd: &[&str]) -> Option<String> {
        let mut all = vec!["exec", self.name.as_str()];
        all.extend_from_slice(cmd);
        let out = run(&all);
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Freezes every process of the container.
    pub fn pause(&self) {
        run_ok(&["pause", &self.name]);
    }

    /// Thaws the container.
    pub fn unpause(&self) {
        run_ok(&["unpause", &self.name]);
    }

    /// Waits until `ready` holds, panicking after `timeout`.
    pub fn wait(&self, what: &str, timeout: Duration, mut ready: impl FnMut(&Self) -> bool) {
        let start = Instant::now();
        while !ready(self) {
            assert!(
                start.elapsed() < timeout,
                "{} did not become {what} within {timeout:?}",
                self.name
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = run(&["rm", "-f", &self.name]);
    }
}

/// An etcd for one test, or an external one from `VERTEP_ETCD_ENDPOINT`.
#[derive(Debug)]
pub struct Etcd {
    pub container: Option<Container>,
    pub endpoint: String,
}

impl Etcd {
    /// A fresh single-member etcd with short election timing, so one-second lease TTLs are
    /// allowed and expire promptly.
    pub fn start() -> Etcd {
        if let Ok(endpoint) = std::env::var("VERTEP_ETCD_ENDPOINT") {
            return Etcd {
                container: None,
                endpoint,
            };
        }
        let c = Container::start(
            "vt-etcd",
            ETCD_IMAGE,
            &["-p", "127.0.0.1::2379"],
            &[
                "etcd",
                "--listen-client-urls",
                "http://0.0.0.0:2379",
                "--advertise-client-urls",
                "http://127.0.0.1:2379",
                "--heartbeat-interval",
                "50",
                "--election-timeout",
                "500",
            ],
        );
        c.wait("healthy", Duration::from_secs(60), |c| {
            c.exec(&["etcdctl", "endpoint", "health"]).is_some()
        });
        let endpoint = format!("http://127.0.0.1:{}", c.host_port(2379));
        Etcd {
            container: Some(c),
            endpoint,
        }
    }

    /// The container, for fault injection; `None` for an external etcd.
    pub fn container(&self) -> Option<&Container> {
        self.container.as_ref()
    }
}

/// A k3s API server (no agent, no add-ons), or an external cluster from `VERTEP_KUBECONFIG`.
#[derive(Debug)]
pub struct K3s {
    pub container: Option<Container>,
    /// The kubeconfig text, its server pointing at the published port.
    pub kubeconfig: String,
}

impl K3s {
    /// A fresh API server.
    pub fn start() -> K3s {
        if let Ok(path) = std::env::var("VERTEP_KUBECONFIG") {
            return K3s {
                container: None,
                kubeconfig: std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{path}: {e}")),
            };
        }
        let c = Container::start(
            "vt-k3s",
            K3S_IMAGE,
            &["-p", "127.0.0.1::6443"],
            &[
                "server",
                "--disable-agent",
                "--disable=traefik,servicelb,metrics-server,local-storage,coredns",
                "--tls-san=127.0.0.1",
            ],
        );
        c.wait("ready", Duration::from_secs(120), |c| {
            c.exec(&["kubectl", "get", "--raw", "/readyz"])
                .is_some_and(|o| o.trim() == "ok")
                && c.exec(&["kubectl", "get", "leases", "-n", "default"])
                    .is_some()
                && c.exec(&[
                    "kubectl",
                    "get",
                    "serviceaccount",
                    "default",
                    "-n",
                    "default",
                ])
                .is_some()
        });
        let config = c
            .exec(&["cat", "/etc/rancher/k3s/k3s.yaml"])
            .expect("the k3s kubeconfig");
        let kubeconfig = config.replace(
            "https://127.0.0.1:6443",
            &format!("https://127.0.0.1:{}", c.host_port(6443)),
        );
        K3s {
            container: Some(c),
            kubeconfig,
        }
    }

    /// One API server shared by the tests of this process that run at the same time; it is
    /// removed when the last of them drops it.
    pub fn shared() -> Arc<K3s> {
        static SHARED: Mutex<Weak<K3s>> = Mutex::new(Weak::new());
        let mut slot = SHARED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(k) = slot.upgrade() {
            return k;
        }
        let k = Arc::new(K3s::start());
        *slot = Arc::downgrade(&k);
        k
    }

    /// The container, for fault injection; `None` for an external cluster.
    pub fn container(&self) -> Option<&Container> {
        self.container.as_ref()
    }
}

/// A user-defined network, removed on drop.
#[derive(Debug)]
pub struct Network {
    pub name: String,
}

impl Network {
    pub fn create(tag: &str) -> Network {
        let name = unique(tag);
        run_ok(&["network", "create", "--label", "vertep-test", &name]);
        Network { name }
    }

    /// Attaches `c` to this network (again).
    pub fn connect(&self, c: &Container) -> bool {
        run(&["network", "connect", &self.name, &c.name])
            .status
            .success()
    }

    /// Detaches `c`: a partition between it and everything only on this network.
    pub fn disconnect(&self, c: &Container) -> bool {
        run(&["network", "disconnect", "--force", &self.name, &c.name])
            .status
            .success()
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        let _ = run(&["network", "rm", &self.name]);
    }
}

/// Runs a docker command to completion; its stdout and stderr when it fails.
pub fn docker_run(args: &[&str]) -> Result<String, String> {
    let out = run(args);
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}
