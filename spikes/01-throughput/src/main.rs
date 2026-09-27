//! Throwaway spike: measure the real-world cost of a "site explorer" crawler.
//!
//! Unit of work = one registered domain (eTLD+1). A visit fetches robots.txt,
//! the homepage, and up to 4 internal pages (most-linked from the homepage).
//! Frontier = FIFO of unvisited registered domains; global seen-set.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error as StdError;
use std::fmt;
use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use hickory_resolver::TokioResolver;
use serde::Serialize;
use shared::crawler::extract_urls;
use shared::url_normalize::{normalize_url, registered_domain};
use texting_robots::Robot;
use tokio::task::JoinSet;
use url::{Host, Url};

// ---------------------------------------------------------------- allocator

struct Counting;
static ALLOC_REQ: AtomicUsize = AtomicUsize::new(0);
static ALLOC_RND: AtomicUsize = AtomicUsize::new(0);
fn rnd(n: usize) -> usize {
    (n + 15) & !15
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            ALLOC_REQ.fetch_add(l.size(), Ordering::Relaxed);
            ALLOC_RND.fetch_add(rnd(l.size()), Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if !p.is_null() {
            ALLOC_REQ.fetch_add(l.size(), Ordering::Relaxed);
            ALLOC_RND.fetch_add(rnd(l.size()), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        ALLOC_REQ.fetch_sub(l.size(), Ordering::Relaxed);
        ALLOC_RND.fetch_sub(rnd(l.size()), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let np = System.realloc(p, l, n);
        if !np.is_null() {
            ALLOC_REQ.fetch_add(n, Ordering::Relaxed);
            ALLOC_REQ.fetch_sub(l.size(), Ordering::Relaxed);
            ALLOC_RND.fetch_add(rnd(n), Ordering::Relaxed);
            ALLOC_RND.fetch_sub(rnd(l.size()), Ordering::Relaxed);
        }
        np
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

// ---------------------------------------------------------------- constants

const UA: &str = "WebCrawlerExplorer-spike/0.1 (+https://github.com/nabi-allenby/web-crawler)";
const ROBOTS_TOKEN: &str = "WebCrawlerExplorer";
const PAGE_CAP: usize = 2 * 1024 * 1024;
const ROBOTS_CAP: usize = 512 * 1024;
const MAX_REDIRECTS: usize = 5;
const MAX_INTERNAL: usize = 4;
const MIN_GAP: Duration = Duration::from_secs(1);
const MAX_HONORED_DELAY_S: f32 = 60.0;
const VISIT_HARD_TIMEOUT: Duration = Duration::from_secs(240);

// ---------------------------------------------------------------- failures

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Fail {
    Dns,
    Connect, // includes TLS handshake failures
    Timeout,
    Http4xx,
    Http429,
    Http5xx,
    Http503,
    RobotsDisallow,
    RobotsFetch5xx,
    RobotsFetchTimeout,
    RobotsFetchOther,
    CrawlDelayTooLong,
    NonHtml,
    BlockedIp,
    TooManyRedirects,
    BadUrl,
    Other,
    VisitTimeout,
}

#[derive(Debug)]
struct DnsFail(String);
impl fmt::Display for DnsFail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dns failure: {}", self.0)
    }
}
impl StdError for DnsFail {}

#[derive(Debug)]
struct BlockedIp(String);
impl fmt::Display for BlockedIp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "blocked non-public ip: {}", self.0)
    }
}
impl StdError for BlockedIp {}

fn classify(e: &reqwest::Error) -> Fail {
    let mut src: Option<&(dyn StdError + 'static)> = Some(e);
    while let Some(s) = src {
        if s.downcast_ref::<BlockedIp>().is_some() {
            return Fail::BlockedIp;
        }
        if s.downcast_ref::<DnsFail>().is_some() {
            return Fail::Dns;
        }
        src = s.source();
    }
    if e.is_timeout() {
        Fail::Timeout
    } else if e.is_connect() {
        Fail::Connect
    } else {
        // Walk once more for io timeouts hidden in the chain.
        let s = format!("{e:?}");
        if s.contains("TimedOut") || s.contains("timed out") {
            Fail::Timeout
        } else {
            Fail::Other
        }
    }
}

// ---------------------------------------------------------------- IP safety

fn v4_public(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xC0) == 64) // CGNAT 100.64/10
        || o[0] >= 240
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (o[1] & 0xFE) == 18))
}

fn v6_public(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4_public(v4);
    }
    let s = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || s[..6].iter().all(|x| *x == 0) // ::/96 (v4-compatible, ::, ::1)
        || (s[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
        || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (s[0] & 0xffc0) == 0xfec0 // site-local (deprecated)
        || (s[0] == 0x2001 && s[1] == 0x0db8)) // documentation
}

fn ip_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v4_public(*v),
        IpAddr::V6(v) => v6_public(*v),
    }
}

struct SafeResolver {
    inner: Arc<TokioResolver>,
}

impl reqwest::dns::Resolve for SafeResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let r = self.inner.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let lookup = r.lookup_ip(host.as_str()).await.map_err(|e| {
                Box::new(DnsFail(format!("{host}: {e}"))) as Box<dyn StdError + Send + Sync>
            })?;
            let addrs: Vec<IpAddr> = lookup.iter().collect();
            if addrs.is_empty() {
                return Err(Box::new(DnsFail(format!("{host}: no addresses")))
                    as Box<dyn StdError + Send + Sync>);
            }
            // Conservative: any non-public address poisons the whole host.
            if let Some(bad) = addrs.iter().find(|a| !ip_public(a)) {
                return Err(Box::new(BlockedIp(format!("{host} -> {bad}")))
                    as Box<dyn StdError + Send + Sync>);
            }
            let it = addrs.into_iter().map(|ip| SocketAddr::new(ip, 0));
            Ok(Box::new(it) as reqwest::dns::Addrs)
        })
    }
}

fn check_url_safe(u: &Url) -> Result<(), Fail> {
    if u.scheme() != "http" && u.scheme() != "https" {
        return Err(Fail::BadUrl);
    }
    match u.host() {
        Some(Host::Ipv4(ip)) if !v4_public(ip) => Err(Fail::BlockedIp),
        Some(Host::Ipv6(ip)) if !v6_public(ip) => Err(Fail::BlockedIp),
        Some(_) => Ok(()),
        None => Err(Fail::BadUrl),
    }
}

// ---------------------------------------------------------------- per-host state

enum RobotsState {
    AllowAll,
    Rules(Robot),
    Unavailable(Fail),
}

struct HostState {
    last: Option<Instant>,
    gap: Duration,
    crawl_delay: Option<f32>,
    robots: Option<Arc<RobotsState>>,
    last_used: Instant,
}

type HostHandle = Arc<tokio::sync::Mutex<HostState>>;

#[derive(Default)]
struct Hosts {
    map: std::sync::Mutex<HashMap<String, HostHandle>>,
}

impl Hosts {
    fn get(&self, key: &str) -> HostHandle {
        let mut m = self.map.lock().unwrap();
        m.entry(key.to_string())
            .or_insert_with(|| {
                Arc::new(tokio::sync::Mutex::new(HostState {
                    last: None,
                    gap: MIN_GAP,
                    crawl_delay: None,
                    robots: None,
                    last_used: Instant::now(),
                }))
            })
            .clone()
    }
    fn sweep(&self, idle: Duration) -> usize {
        let mut m = self.map.lock().unwrap();
        m.retain(|_, h| {
            if Arc::strong_count(h) > 1 {
                return true;
            }
            match h.try_lock() {
                Ok(g) => g.last_used.elapsed() < idle,
                Err(_) => true,
            }
        });
        m.len()
    }
}

fn host_key(u: &Url) -> String {
    let h = u.host_str().unwrap_or("").to_ascii_lowercase();
    match u.port() {
        Some(p) => format!("{h}:{p}"),
        None => h,
    }
}

async fn wait_turn(st: &mut HostState) {
    if let Some(last) = st.last {
        let ready = last + st.gap;
        if ready > Instant::now() {
            tokio::time::sleep_until(ready.into()).await;
        }
    }
}

// ---------------------------------------------------------------- HTTP

struct Ctx {
    client: reqwest::Client,
    hosts: Hosts,
}

struct Raw {
    status: u16,
    location: Option<String>,
    non_html: bool,
    body: Vec<u8>,
    wire: usize,
    capped: bool,
}

#[derive(Serialize, Clone)]
struct Event {
    kind: &'static str, // robots | page
    host: String,
    status: Option<u16>,
    fail: Option<Fail>,
    wire: usize,
    decoded: usize,
    ms: u64,
    capped: bool,
}

/// One HTTP request, no redirect following, no politeness (caller handles it).
async fn raw_get(client: &reqwest::Client, url: &Url, cap: usize, html_only: bool) -> Result<Raw, Fail> {
    check_url_safe(url)?;
    let resp = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "gzip")
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml;q=0.9,text/plain;q=0.5,*/*;q=0.1",
        )
        .send()
        .await
        .map_err(|e| classify(&e))?;
    let status = resp.status().as_u16();
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    if !(200..300).contains(&status) {
        return Ok(Raw { status, location, non_html: false, body: vec![], wire: 0, capped: false });
    }
    if html_only {
        if let Some(ct) = resp.headers().get(reqwest::header::CONTENT_TYPE) {
            let ct = ct.to_str().unwrap_or_default().to_ascii_lowercase();
            if !(ct.starts_with("text/html") || ct.starts_with("application/xhtml")) {
                return Ok(Raw { status, location, non_html: true, body: vec![], wire: 0, capped: false });
            }
        }
    }
    let gz = resp
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("gzip") || s.eq_ignore_ascii_case("x-gzip"))
        .unwrap_or(false);
    let mut stream = resp.bytes_stream();
    let mut wire = 0usize;
    let mut capped = false;
    let mut plain: Vec<u8> = Vec::new();
    let mut dec = flate2::write::GzDecoder::new(Vec::new());
    let mut gz_broken = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| classify(&e))?;
        wire += chunk.len();
        let cur = if gz {
            if !gz_broken && dec.write_all(&chunk).is_err() {
                gz_broken = true;
            }
            dec.get_ref().len()
        } else {
            plain.extend_from_slice(&chunk);
            plain.len()
        };
        if cur >= cap {
            capped = true;
            break;
        }
    }
    let mut body = if gz {
        let _ = dec.flush();
        std::mem::take(dec.get_mut())
    } else {
        plain
    };
    body.truncate(cap);
    Ok(Raw { status, location, non_html: false, body, wire, capped })
}

fn ev_from(kind: &'static str, url: &Url, r: &Result<Raw, Fail>, t: Instant) -> Event {
    let ms = t.elapsed().as_millis() as u64;
    match r {
        Ok(raw) => Event {
            kind,
            host: host_key(url),
            status: Some(raw.status),
            fail: None,
            wire: raw.wire,
            decoded: raw.body.len(),
            ms,
            capped: raw.capped,
        },
        Err(f) => Event {
            kind,
            host: host_key(url),
            status: None,
            fail: Some(f.clone()),
            wire: 0,
            decoded: 0,
            ms,
            capped: false,
        },
    }
}

/// Fetch and parse robots.txt for `host`, holding that host's lock (`st`).
/// Returns (state, scheme that worked).
async fn fetch_robots(
    ctx: &Ctx,
    host: &str,
    schemes: &[&str],
    st: &mut HostState,
    ev: &mut Vec<Event>,
) -> (Arc<RobotsState>, Option<String>, Option<String>) {
    'scheme: for (si, scheme) in schemes.iter().enumerate() {
        let mut url = match Url::parse(&format!("{scheme}://{host}/robots.txt")) {
            Ok(u) => u,
            Err(_) => return (Arc::new(RobotsState::Unavailable(Fail::BadUrl)), None, None),
        };
        let mut hops = 0;
        loop {
            let t = Instant::now();
            let res = if host_key(&url) == host {
                wait_turn(st).await;
                let r = raw_get(&ctx.client, &url, ROBOTS_CAP, false).await;
                st.last = Some(Instant::now());
                r
            } else {
                // Cross-host robots redirect: respect that host's spacing too.
                let other = ctx.hosts.get(&host_key(&url));
                let locked = tokio::time::timeout(Duration::from_secs(30), other.lock()).await;
                let r = match locked {
                    Ok(mut g) => {
                        if url.path() == "/robots.txt" {
                            if let Some(r) = &g.robots {
                                // Already know this host's robots: reuse, no refetch.
                                return (r.clone(), Some(scheme.to_string()), None);
                            }
                        }
                        wait_turn(&mut g).await;
                        let r = raw_get(&ctx.client, &url, ROBOTS_CAP, false).await;
                        g.last = Some(Instant::now());
                        g.last_used = Instant::now();
                        r
                    }
                    Err(_) => Err(Fail::RobotsFetchOther),
                };
                r
            };
            ev.push(ev_from("robots", &url, &res, t));
            match res {
                Err(Fail::Connect) if hops == 0 && si + 1 < schemes.len() => continue 'scheme,
                Err(Fail::Timeout) => return (Arc::new(RobotsState::Unavailable(Fail::RobotsFetchTimeout)), None, None),
                Err(f @ (Fail::Dns | Fail::Connect | Fail::BlockedIp)) => {
                    return (Arc::new(RobotsState::Unavailable(f)), None, None)
                }
                Err(_) => return (Arc::new(RobotsState::Unavailable(Fail::RobotsFetchOther)), None, None),
                Ok(raw) => match raw.status {
                    200..=299 => {
                        let robot = match Robot::new(ROBOTS_TOKEN, &raw.body) {
                            Ok(r) => RobotsState::Rules(r),
                            Err(_) => RobotsState::AllowAll,
                        };
                        return (Arc::new(robot), Some(scheme.to_string()), Some(host_key(&url)));
                    }
                    300..=399 => {
                        hops += 1;
                        let next = raw.location.as_deref().and_then(|l| url.join(l).ok());
                        match next {
                            Some(n) if hops <= MAX_REDIRECTS => url = n,
                            // Too many / broken redirects: treat like 404 (allow all).
                            _ => return (Arc::new(RobotsState::AllowAll), Some(scheme.to_string()), None),
                        }
                    }
                    429 => return (Arc::new(RobotsState::Unavailable(Fail::Http429)), None, None),
                    400..=499 => return (Arc::new(RobotsState::AllowAll), Some(scheme.to_string()), Some(host_key(&url))),
                    500..=599 => return (Arc::new(RobotsState::Unavailable(Fail::RobotsFetch5xx)), None, None),
                    _ => return (Arc::new(RobotsState::Unavailable(Fail::RobotsFetchOther)), None, None),
                },
            }
        }
    }
    (Arc::new(RobotsState::Unavailable(Fail::Connect)), None, None)
}

/// Ensure robots for the URL's host is known. Returns (state, crawl_delay, scheme used).
async fn ensure_robots(
    ctx: &Ctx,
    host: &str,
    schemes: &[&str],
    ev: &mut Vec<Event>,
) -> (Arc<RobotsState>, Option<f32>, Option<String>) {
    let h = ctx.hosts.get(host);
    let mut st = h.lock().await;
    st.last_used = Instant::now();
    if let Some(r) = &st.robots {
        return (r.clone(), st.crawl_delay, None);
    }
    let (state, scheme, final_host) = fetch_robots(ctx, host, schemes, &mut st, ev).await;
    let delay = match &*state {
        RobotsState::Rules(r) => r.delay,
        _ => None,
    };
    apply_robots(&mut st, delay);
    let arc = state;
    st.robots = Some(arc.clone());
    // A cross-host robots redirect (e.g. apex -> www) fetched the other host's
    // robots.txt; cache it there too so it is fetched once per host.
    if let Some(fh) = final_host.filter(|fh| fh != host) {
        let other = ctx.hosts.get(&fh);
        let locked = tokio::time::timeout(Duration::from_secs(30), other.lock()).await;
        if let Ok(mut g) = locked {
            if g.robots.is_none() {
                g.robots = Some(arc.clone());
                apply_robots(&mut g, delay);
            }
        }
    }
    (arc, st.crawl_delay, scheme)
}

fn apply_robots(st: &mut HostState, delay: Option<f32>) {
    st.crawl_delay = delay;
    if let Some(d) = delay {
        if d.is_finite() && d > 1.0 {
            st.gap = Duration::from_secs_f32(d.min(MAX_HONORED_DELAY_S));
        }
    }
}

struct PageOk {
    final_url: Url,
    html: String,
}

/// Fetch a page following redirects manually; robots + politeness on every hop.
async fn fetch_page(ctx: &Ctx, start: Url, ev: &mut Vec<Event>) -> Result<PageOk, Fail> {
    let mut url = start;
    for _hop in 0..=MAX_REDIRECTS {
        check_url_safe(&url)?;
        let hk = host_key(&url);
        let scheme = url.scheme().to_string();
        let (robots, delay, _) = ensure_robots(ctx, &hk, &[scheme.as_str()], ev).await;
        match &*robots {
            RobotsState::Unavailable(f) => return Err(f.clone()),
            RobotsState::AllowAll => {}
            RobotsState::Rules(r) => {
                if !r.allowed(url.as_str()) {
                    return Err(Fail::RobotsDisallow);
                }
            }
        }
        if delay.map(|d| d > MAX_HONORED_DELAY_S).unwrap_or(false) {
            return Err(Fail::CrawlDelayTooLong);
        }
        let h = ctx.hosts.get(&hk);
        let (res, t_req) = {
            let mut st = h.lock().await;
            wait_turn(&mut st).await;
            let t_req = Instant::now();
            let r = raw_get(&ctx.client, &url, PAGE_CAP, true).await;
            st.last = Some(Instant::now());
            st.last_used = Instant::now();
            (r, t_req)
        };
        ev.push(ev_from("page", &url, &res, t_req));
        let raw = res?;
        match raw.status {
            200..=299 => {
                if raw.non_html {
                    return Err(Fail::NonHtml);
                }
                return Ok(PageOk { final_url: url, html: String::from_utf8_lossy(&raw.body).into_owned() });
            }
            300..=399 => match raw.location.as_deref().and_then(|l| url.join(l).ok()) {
                Some(n) => url = n,
                None => return Err(Fail::Other),
            },
            429 => return Err(Fail::Http429),
            503 => return Err(Fail::Http503),
            400..=499 => return Err(Fail::Http4xx),
            500..=599 => return Err(Fail::Http5xx),
            _ => return Err(Fail::Other),
        }
    }
    Err(Fail::TooManyRedirects)
}

// ---------------------------------------------------------------- parsing

fn thread_cpu_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe {
        libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts);
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

struct Parsed {
    ext: HashSet<String>,
    internal: Vec<(String, String)>, // (normalized name, fetch url without fragment)
    n_links: usize,
    parse_cpu_us: u64,
    classify_cpu_us: u64,
}

fn parse_page(html: String, base: String, site: String) -> Parsed {
    let c0 = thread_cpu_ns();
    let links = extract_urls(&html, &base);
    let c1 = thread_cpu_ns();
    let mut ext = HashSet::new();
    let mut internal = Vec::new();
    for l in &links {
        let Ok(mut u) = Url::parse(l) else { continue };
        if !matches!(u.host(), Some(Host::Domain(_))) {
            continue;
        }
        let n = normalize_url(l);
        let Some(rd) = registered_domain(&n.host) else { continue };
        if rd == site {
            u.set_fragment(None);
            internal.push((n.name, u.to_string()));
        } else {
            ext.insert(rd);
        }
    }
    let c2 = thread_cpu_ns();
    Parsed {
        ext,
        internal,
        n_links: links.len(),
        parse_cpu_us: (c1 - c0) / 1000,
        classify_cpu_us: (c2 - c1) / 1000,
    }
}

// ---------------------------------------------------------------- visit

#[derive(Serialize, Default)]
struct PageRec {
    url: String,
    ok: bool,
    fail: Option<Fail>,
    decoded: usize,
    n_links: usize,
    parse_cpu_us: u64,
    classify_cpu_us: u64,
    ext_domains: usize,
}

#[derive(Serialize, Default)]
struct VisitRec {
    domain: String,
    t_start: f64,
    t_end: f64,
    dur_ms: u64,
    outcome: Option<Fail>, // None = completed (homepage fetched)
    scheme: Option<String>,
    homepage_only_crawl_delay: bool,
    crawl_delay: Option<f32>,
    site_domain: Option<String>,
    pages_attempted: usize,
    pages_ok: usize,
    stopped_throttled: bool,
    wire_bytes: usize,
    decoded_bytes: usize,
    requests: usize,
    robots_requests: usize,
    ext_domains: usize,
    ext_weight_sum: u32,
    new_domains: usize,
    seen_after: usize,
    frontier_after: usize,
    pages: Vec<PageRec>,
    events: Vec<Event>,
    #[serde(skip)]
    ext_list: Vec<String>,
}

async fn visit(ctx: Arc<Ctx>, domain: String, t0: Instant) -> VisitRec {
    let start = Instant::now();
    let mut rec = VisitRec { domain: domain.clone(), t_start: (start - t0).as_secs_f64(), ..Default::default() };
    let mut ev: Vec<Event> = Vec::new();
    let r = tokio::time::timeout(VISIT_HARD_TIMEOUT, visit_inner(&ctx, &domain, &mut rec, &mut ev)).await;
    if r.is_err() {
        rec.outcome = Some(Fail::VisitTimeout);
    }
    rec.requests = ev.len();
    rec.robots_requests = ev.iter().filter(|e| e.kind == "robots").count();
    rec.wire_bytes = ev.iter().map(|e| e.wire).sum();
    rec.decoded_bytes = ev.iter().map(|e| e.decoded).sum();
    rec.events = ev;
    rec.dur_ms = start.elapsed().as_millis() as u64;
    rec.t_end = (Instant::now() - t0).as_secs_f64();
    rec
}

async fn run_parse(html: String, base: &Url, site: &str) -> Parsed {
    let b = base.to_string();
    let s = site.to_string();
    tokio::task::spawn_blocking(move || parse_page(html, b, s)).await.expect("parse task")
}

async fn visit_inner(ctx: &Ctx, domain: &str, rec: &mut VisitRec, ev: &mut Vec<Event>) {
    let host = domain.to_ascii_lowercase();
    // 1. robots.txt for the apex host, https with http fallback on connect failure.
    let (robots, delay, scheme) = ensure_robots(ctx, &host, &["https", "http"], ev).await;
    rec.crawl_delay = delay;
    if let RobotsState::Unavailable(f) = &*robots {
        rec.outcome = Some(f.clone());
        return;
    }
    if delay.map(|d| d > MAX_HONORED_DELAY_S).unwrap_or(false) {
        rec.outcome = Some(Fail::CrawlDelayTooLong);
        return;
    }
    let scheme = scheme.unwrap_or_else(|| "https".into());
    rec.scheme = Some(scheme.clone());
    let homepage_only = delay.map(|d| d > 10.0).unwrap_or(false);
    rec.homepage_only_crawl_delay = homepage_only;

    // 2. homepage
    let home = match Url::parse(&format!("{scheme}://{host}/")) {
        Ok(u) => u,
        Err(_) => {
            rec.outcome = Some(Fail::BadUrl);
            return;
        }
    };
    rec.pages_attempted = 1;
    let hp = match fetch_page(ctx, home.clone(), ev).await {
        Ok(p) => p,
        Err(f) => {
            rec.pages.push(PageRec { url: home.to_string(), ok: false, fail: Some(f.clone()), ..Default::default() });
            rec.outcome = Some(f);
            return;
        }
    };
    let site = hp
        .final_url
        .host_str()
        .and_then(|h| registered_domain(&h.to_ascii_uppercase()))
        .unwrap_or_else(|| domain.to_string());
    rec.site_domain = Some(site.clone());
    let decoded = hp.html.len();
    let parsed = run_parse(hp.html, &hp.final_url, &site).await;
    rec.pages_ok = 1;
    rec.pages.push(PageRec {
        url: hp.final_url.to_string(),
        ok: true,
        fail: None,
        decoded,
        n_links: parsed.n_links,
        parse_cpu_us: parsed.parse_cpu_us,
        classify_cpu_us: parsed.classify_cpu_us,
        ext_domains: parsed.ext.len(),
    });
    let mut weights: HashMap<String, u32> = HashMap::new();
    for d in &parsed.ext {
        if d != domain {
            *weights.entry(d.clone()).or_default() += 1;
        }
    }

    // 3. pick up to 4 internal pages, most frequently linked from homepage.
    if !homepage_only {
        let skip: HashSet<String> = [normalize_url(home.as_str()).name, normalize_url(hp.final_url.as_str()).name]
            .into_iter()
            .collect();
        let mut counts: HashMap<String, (usize, usize, String)> = HashMap::new();
        for (i, (name, u)) in parsed.internal.into_iter().enumerate() {
            if skip.contains(&name) {
                continue;
            }
            counts.entry(name).or_insert((0, i, u)).0 += 1;
        }
        let mut cands: Vec<(usize, usize, String)> = counts.into_values().collect();
        cands.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, _, u) in cands.into_iter().take(MAX_INTERNAL) {
            let Ok(url) = Url::parse(&u) else { continue };
            rec.pages_attempted += 1;
            match fetch_page(ctx, url.clone(), ev).await {
                Ok(p) => {
                    let decoded = p.html.len();
                    let pp = run_parse(p.html, &p.final_url, &site).await;
                    rec.pages_ok += 1;
                    for d in &pp.ext {
                        if d != domain {
                            *weights.entry(d.clone()).or_default() += 1;
                        }
                    }
                    rec.pages.push(PageRec {
                        url: p.final_url.to_string(),
                        ok: true,
                        fail: None,
                        decoded,
                        n_links: pp.n_links,
                        parse_cpu_us: pp.parse_cpu_us,
                        classify_cpu_us: pp.classify_cpu_us,
                        ext_domains: pp.ext.len(),
                    });
                }
                Err(f) => {
                    let stop = matches!(f, Fail::Http429 | Fail::Http503);
                    rec.pages.push(PageRec { url: u.clone(), ok: false, fail: Some(f), ..Default::default() });
                    if stop {
                        rec.stopped_throttled = true;
                        break;
                    }
                }
            }
        }
    }
    rec.ext_domains = weights.len();
    rec.ext_weight_sum = weights.values().sum();
    rec.ext_list = weights.into_keys().collect();
    rec.outcome = None;
}

// ---------------------------------------------------------------- main

fn peak_rss_bytes() -> i64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe {
        libc::getrusage(libc::RUSAGE_SELF, &mut ru);
    }
    ru.ru_maxrss as i64 // bytes on macOS
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn StdError>> {
    let args: Vec<String> = std::env::args().collect();
    let conc: usize = arg(&args, "--concurrency").map(|s| s.parse().unwrap()).unwrap_or(16);
    let dur = Duration::from_secs(arg(&args, "--secs").map(|s| s.parse().unwrap()).unwrap_or(480));
    let out = arg(&args, "--out").unwrap_or_else(|| "out".into());
    let run = arg(&args, "--run").unwrap_or_else(|| "A".into());
    let seeds = arg(&args, "--seeds");
    let resume = arg(&args, "--resume"); // prefix of <run>_frontier.txt / <run>_seen.txt

    let resolver = Arc::new(TokioResolver::builder_tokio()?.build());
    let client = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(SafeResolver { inner: resolver }))
        .pool_idle_timeout(Duration::from_secs(20))
        .pool_max_idle_per_host(1)
        .build()?;
    let ctx = Arc::new(Ctx { client, hosts: Hosts::default() });

    let mut frontier: VecDeque<String> = VecDeque::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(prefix) = &resume {
        for l in std::fs::read_to_string(format!("{prefix}_seen.txt"))?.lines() {
            seen.insert(l.to_string());
        }
        for l in std::fs::read_to_string(format!("{prefix}_frontier.txt"))?.lines() {
            frontier.push_back(l.to_string());
        }
        eprintln!("resumed: frontier={} seen={}", frontier.len(), seen.len());
    } else {
        let path = seeds.expect("--seeds or --resume");
        for l in std::fs::read_to_string(path)?.lines() {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let d = registered_domain(&l.to_ascii_uppercase()).expect("seed domain");
            if seen.insert(d.clone()) {
                frontier.push_back(d);
            }
        }
    }
    let seen_at_start = seen.len();

    let mut vf = std::io::BufWriter::new(std::fs::File::create(format!("{out}/{run}_visits.jsonl"))?);
    let t0 = Instant::now();
    let deadline = t0 + dur;
    let mut set: JoinSet<VisitRec> = JoinSet::new();
    let mut attempted = 0usize;
    let mut finished = 0usize;
    let mut completed = 0usize;
    let mut throttled = 0usize;
    let mut last_progress = Instant::now();
    let mut idle_waits = 0usize;

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        while set.len() < conc {
            match frontier.pop_front() {
                Some(d) => {
                    attempted += 1;
                    set.spawn(visit(ctx.clone(), d, t0));
                }
                None => {
                    idle_waits += 1;
                    break;
                }
            }
        }
        if set.is_empty() {
            eprintln!("frontier exhausted");
            break;
        }
        tokio::select! {
            r = set.join_next() => {
                let Some(r) = r else { continue };
                let mut rec = match r { Ok(v) => v, Err(e) => { eprintln!("task error: {e}"); continue } };
                finished += 1;
                if rec.outcome.is_none() { completed += 1; }
                if rec.stopped_throttled || matches!(rec.outcome, Some(Fail::Http429) | Some(Fail::Http503)) { throttled += 1; }
                let mut new = 0;
                for d in rec.ext_list.drain(..) {
                    if seen.insert(d.clone()) { frontier.push_back(d); new += 1; }
                }
                rec.new_domains = new;
                rec.seen_after = seen.len();
                rec.frontier_after = frontier.len();
                serde_json::to_writer(&mut vf, &rec)?;
                vf.write_all(b"\n")?;
            }
            _ = tokio::time::sleep_until(deadline.into()) => { break; }
        }
        if last_progress.elapsed() > Duration::from_secs(30) {
            last_progress = Instant::now();
            let el = t0.elapsed().as_secs_f64();
            let hosts = ctx.hosts.sweep(Duration::from_secs(120));
            eprintln!(
                "[{:>5.0}s] attempted={} finished={} completed={} ({:.2} sites/s) throttled={} frontier={} seen={} hosts={} rss={}MB heap={}MB",
                el, attempted, finished, completed, completed as f64 / el, throttled, frontier.len(), seen.len(), hosts,
                peak_rss_bytes() / 1_048_576, ALLOC_RND.load(Ordering::Relaxed) / 1_048_576
            );
        }
    }
    let elapsed = t0.elapsed().as_secs_f64();
    let in_flight = set.len();
    set.abort_all();
    while set.join_next().await.is_some() {}
    vf.flush()?;

    // Memory per entry: rebuild copies of frontier/seen and measure heap delta.
    let avg_len = seen.iter().map(|s| s.len()).sum::<usize>() as f64 / seen.len().max(1) as f64;
    let (r0, q0) = (ALLOC_RND.load(Ordering::Relaxed), ALLOC_REQ.load(Ordering::Relaxed));
    let seen_copy: HashSet<String> = seen.iter().cloned().collect();
    let (r1, q1) = (ALLOC_RND.load(Ordering::Relaxed), ALLOC_REQ.load(Ordering::Relaxed));
    let fr_copy: VecDeque<String> = frontier.iter().cloned().collect();
    let (r2, q2) = (ALLOC_RND.load(Ordering::Relaxed), ALLOC_REQ.load(Ordering::Relaxed));
    let hashes: HashSet<u64> = seen.iter().map(|s| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut h);
        h.finish()
    }).collect();
    let r3 = ALLOC_RND.load(Ordering::Relaxed);
    let seen_n = seen_copy.len().max(1) as f64;
    let fr_n = fr_copy.len().max(1) as f64;

    std::fs::write(format!("{out}/{run}_seen.txt"), seen.iter().cloned().collect::<Vec<_>>().join("\n"))?;
    std::fs::write(format!("{out}/{run}_frontier.txt"), frontier.iter().cloned().collect::<Vec<_>>().join("\n"))?;

    let summary = serde_json::json!({
        "run": run,
        "concurrency": conc,
        "duration_s": elapsed,
        "resumed": resume.is_some(),
        "attempted": attempted,
        "finished": finished,
        "completed": completed,
        "in_flight_aborted": in_flight,
        "throttled_visits": throttled,
        "frontier_idle_events": idle_waits,
        "seen_at_start": seen_at_start,
        "seen_end": seen.len(),
        "frontier_end": frontier.len(),
        "avg_domain_len": avg_len,
        "seen_hashset_bytes_per_entry_rounded16": (r1 - r0) as f64 / seen_n,
        "seen_hashset_bytes_per_entry_requested": (q1 - q0) as f64 / seen_n,
        "frontier_vecdeque_bytes_per_entry_rounded16": (r2 - r1) as f64 / fr_n,
        "frontier_vecdeque_bytes_per_entry_requested": (q2 - q1) as f64 / fr_n,
        "seen_u64hash_bytes_per_entry": (r3 - r2) as f64 / seen_n,
        "peak_rss_bytes_getrusage": peak_rss_bytes(),
        "heap_live_bytes_end": r0,
    });
    drop((seen_copy, fr_copy, hashes));
    std::fs::write(format!("{out}/{run}_summary.json"), serde_json::to_string_pretty(&summary)?)?;
    eprintln!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
