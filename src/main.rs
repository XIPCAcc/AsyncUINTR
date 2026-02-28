/**
 * UINTR-BI: User-Level Interrupt Benchmark for Bidirectional Communication
 * 
 * 该项目实现了基于用户态中断（UINTR）的双向通信基准测试，
 * 用于测量服务器与客户端之间的通信性能。
 * 
 * 主要功能：
 * - 基于用户态中断的双向通信
 * - 详细的性能统计（延迟、吞吐量、CPU使用率等）
 * - 支持可配置的消息数量
 * - 线程安全的中断处理
 * 
 * 使用方法：
 * ```
 * cargo run -- <message_count>
 * ```
 * 
 * 性能指标：
 * - 消息速率（msg/s）
 * - 数据速率（MB/s）
 * - 平均延迟（us）
 * - 最小/最大延迟（us）
 * - 延迟分布（P50、P90、P99）
 * - CPU使用率（%）
 * 
 * 技术实现：
 * - Rust语言实现主要逻辑
 * - C语言实现中断处理程序
 * - 使用UINTR系统调用进行用户态中断管理
 * - 内联汇编实现senduipi、stui、clui指令
 */

#[warn(unused)]
use core::arch::asm;
use libc::{c_int, c_long, sleep, syscall};
use std::os::unix::io::{RawFd, AsRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use tokio::sync::Notify;

// 系统调用号定义
// uintr_wait系统调用实现
pub fn uintr_wait(flags: c_int) -> Result<(), String> {
    let result = unsafe {
        syscall(__NR_UINTR_WAIT as c_long, flags as c_long)
    };
    
    if result < 0 {
        Err(format!("uintr_wait failed with error code: {}", result))
    } else {
        Ok(())
    }
}

static I: std::sync::Mutex<u32> = std::sync::Mutex::new(0);

// UINTR栈帧结构（必须与内核一致）
#[repr(C)]
pub struct UintrFrame {
    pub rip: u64,    // 中断返回地址
    pub rflags: u64, // 标志寄存器
    pub rsp: u64,    // 栈指针
}

// Syscall numbers for UINTR
const __NR_UINTR_REGISTER_HANDLER: c_long = 471;
const __NR_UINTR_UNREGISTER_HANDLER: c_long = 472;
const __NR_UINTR_CREATE_FD: c_long = 473;
const __NR_UINTR_REGISTER_SENDER: c_long = 474;
const __NR_UINTR_UNREGISTER_SENDER: c_long = 475;
const __NR_UINTR_WAIT: c_long = 476;

// 声明C语言中断处理程序和辅助函数
unsafe extern "C" {
    pub fn server_ui_handler(ui_frame: *mut UintrFrame, vector: u64);
    pub fn client_ui_handler(ui_frame: *mut UintrFrame, vector: u64);
    pub fn get_server_uintr_received() -> c_int;
    pub fn get_client_uintr_received() -> c_int;
    pub fn set_server_uintr_received(value: c_int);
    pub fn set_client_uintr_received(value: c_int);
}

// Rust回调函数，供C代码调用
#[unsafe(no_mangle)]
pub extern "C" fn rust_interrupt_callback(handler_name: *const libc::c_char, vector: u64) {
    unsafe {
        match vector {
            SERVER_TOKEN => {
                if SERVER_INITIALIZED {
                    if let Some(ref token) = SERVER_TOKEN_OBJ {
                        let mut interrupt_received = token.inner.interrupt_received.lock().unwrap();
                        *interrupt_received = true;
                    }
                }
            }
            CLIENT_TOKEN => {
                if CLIENT_INITIALIZED {
                    if let Some(ref token) = CLIENT_TOKEN_OBJ {
                        let mut interrupt_received = token.inner.interrupt_received.lock().unwrap();
                        *interrupt_received = true;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Sends a user interrupt to the specified index
///
/// # Safety
/// - The caller must ensure that the index is valid and registered
/// - Sending to an invalid index may lead to undefined behavior
pub unsafe fn senduipi(index: u64) {
    unsafe {
        asm!(
            "senduipi {0}",
            in(reg) index,   // Parameter passed through general register
            options(nostack, nomem)
        );
    }
}

/// Enables user interrupts
///
/// # Safety
/// - The caller must ensure that user interrupts are properly initialized
/// - Enabling user interrupts without proper setup may lead to unexpected behavior
pub unsafe fn stui() {
    unsafe {
        asm!("stui");
    }
}

/// Disables user interrupts
///
/// # Safety
/// - The caller must ensure that disabling user interrupts won't break critical sections
/// - This function should be called in pairs with `stui`
pub unsafe fn clui() {
    unsafe {
        asm!("clui");
    }
}

/// User interrupt return instruction
///
/// # Safety
/// - This function should only be called from within a user interrupt handler
/// - It must be the last instruction executed in the interrupt handler
pub unsafe fn uiret() {
    unsafe {
        asm!("uiret", options(noreturn));
    }
}

// Safe wrappers for syscalls
fn uintr_register_handler(
    handler: unsafe extern "C" fn(*mut UintrFrame, u64),
    flags: c_int,
) -> Result<c_int, String> {
    let result = unsafe { syscall(__NR_UINTR_REGISTER_HANDLER, handler, flags) as c_int };
    if result < 0 {
        Err(format!(
            "uintr_register_handler failed: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(result)
    }
}

fn uintr_create_fd(vector: c_int, flags: c_int) -> Result<RawFd, String> {
    let result = unsafe { syscall(__NR_UINTR_CREATE_FD, vector, flags) as RawFd };
    if result < 0 {
        Err(format!(
            "uintr_create_fd failed: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(result)
    }
}

fn uintr_register_sender(fd: RawFd, flags: c_int) -> Result<c_int, String> {
    let result = unsafe { syscall(__NR_UINTR_REGISTER_SENDER, fd, flags) as c_int };
    if result < 0 {
        Err(format!(
            "uintr_register_sender failed: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(result)
    }
}

// 通过Unix Domain Socket发送文件描述符
fn send_fd(socket: &UnixStream, fd: RawFd) -> Result<(), String> {
    unsafe {
        use libc::{msghdr, iovec, sendmsg, CMSG_FIRSTHDR, CMSG_DATA, SOL_SOCKET, SCM_RIGHTS};
        
        let mut buf = [0u8; 1];
        let mut iov = iovec {
            iov_base: buf.as_mut_ptr() as *mut libc::c_void,
            iov_len: 1,
        };
        
        let fd_size = std::mem::size_of::<RawFd>();
        let cmsg_space_size = libc::CMSG_SPACE(fd_size as u32) as usize;
        let mut cmsg_space: Vec<u8> = vec![0; cmsg_space_size];
        
        let msg = msghdr {
            msg_name: std::ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: cmsg_space.as_mut_ptr() as *mut libc::c_void,
            msg_controllen: cmsg_space.len(),
            msg_flags: 0,
        };
        
        let cmsg = CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = SOL_SOCKET;
        (*cmsg).cmsg_type = SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(fd_size as u32) as usize;
        
        let data = CMSG_DATA(cmsg);
        *(data as *mut RawFd) = fd;
        
        let result = sendmsg(socket.as_raw_fd(), &msg, 0);
        if result < 0 {
            return Err(format!("send_fd failed: {}", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

// 通过Unix Domain Socket接收文件描述符
fn recv_fd(socket: &UnixStream) -> Result<RawFd, String> {
    unsafe {
        use libc::{msghdr, iovec, recvmsg, CMSG_FIRSTHDR, CMSG_DATA, SOL_SOCKET, SCM_RIGHTS};
        
        let mut buf = [0u8; 1];
        let mut iov = iovec {
            iov_base: buf.as_mut_ptr() as *mut libc::c_void,
            iov_len: 1,
        };
        
        let fd_size = std::mem::size_of::<RawFd>();
        let cmsg_space_size = libc::CMSG_SPACE(fd_size as u32) as usize;
        let mut cmsg_space: Vec<u8> = vec![0; cmsg_space_size];
        
        let mut msg = msghdr {
            msg_name: std::ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: cmsg_space.as_mut_ptr() as *mut libc::c_void,
            msg_controllen: cmsg_space.len(),
            msg_flags: 0,
        };
        
        let result = recvmsg(socket.as_raw_fd(), &mut msg, 0);
        if result < 0 {
            return Err(format!("recv_fd failed: {}", std::io::Error::last_os_error()));
        }
        
        let cmsg = CMSG_FIRSTHDR(&msg);
        if cmsg.is_null() || (*cmsg).cmsg_level != SOL_SOCKET || (*cmsg).cmsg_type != SCM_RIGHTS {
            return Err("recv_fd: no file descriptor received".to_string());
        }
        
        let data = CMSG_DATA(cmsg);
        let fd = *(data as *const RawFd);
        Ok(fd)
    }
}

// 定义向量/令牌常量
const SERVER_TOKEN: u64 = 0;
const CLIENT_TOKEN: u64 = 1;

// 全局 UintrToken 实例
static mut SERVER_TOKEN_OBJ: Option<UintrToken> = None;
static mut CLIENT_TOKEN_OBJ: Option<UintrToken> = None;

// 全局初始化标志
static mut SERVER_INITIALIZED: bool = false;
static mut CLIENT_INITIALIZED: bool = false;

// 供 Tokio IO Driver 调用的函数
#[unsafe(no_mangle)]
pub extern "C" fn check_uintr_pending() -> bool {
    unsafe {
        let server_pending = SERVER_TOKEN_OBJ.as_ref().map(|token| {
            *token.inner.interrupt_received.lock().unwrap()
        }).unwrap_or(false);
        
        let client_pending = CLIENT_TOKEN_OBJ.as_ref().map(|token| {
            *token.inner.interrupt_received.lock().unwrap()
        }).unwrap_or(false);
        
        let has_pending = server_pending || client_pending;
        has_pending
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn process_uintr_wakers() -> u32 {
    unsafe {
        let mut waker_count = 0;
        
        if let Some(ref token) = SERVER_TOKEN_OBJ {
            let should_wake = {
                let mut interrupt_received = token.inner.interrupt_received.lock().unwrap();
                if *interrupt_received {
                    *interrupt_received = false;
                    {
                        let mut pending = token.inner.pending.lock().unwrap();
                        *pending = true;
                    }
                    true
                } else {
                    false
                }
            };
            
            if should_wake {
                if let Some(waker) = token.inner.waker.lock().unwrap().take() {
                    waker.wake();
                    waker_count += 1;
                }
            }
        }
        
        if let Some(ref token) = CLIENT_TOKEN_OBJ {
            let should_wake = {
                let mut interrupt_received = token.inner.interrupt_received.lock().unwrap();
                if *interrupt_received {
                    *interrupt_received = false;
                    {
                        let mut pending = token.inner.pending.lock().unwrap();
                        *pending = true;
                    }
                    true
                } else {
                    false
                }
            };
            
            if should_wake {
                if let Some(waker) = token.inner.waker.lock().unwrap().take() {
                    waker.wake();
                    waker_count += 1;
                }
            }
        }
        
        waker_count
    }
}

/// 表示某个 UINTR 中断源的句柄
#[derive(Clone)]
pub struct UintrToken {
    inner: Arc<Inner>,
    name: String,
}

struct Inner {
    /// 中断是否已经到达（用于 check_uintr_pending）
    interrupt_received: Mutex<bool>,
    /// 是否已经收到一次中断（用于 UintrFuture::poll）
    pending: Mutex<bool>,
    /// 当前在等这个中断的任务的 waker（最多一个）
    waker: Mutex<Option<Waker>>,
}

impl UintrToken {
    pub fn new(name: &str) -> Self {
        Self {
            inner: Arc::new(Inner {
                interrupt_received: Mutex::new(false),
                pending: Mutex::new(false),
                waker: Mutex::new(None),
            }),
            name: name.to_string(),
        }
    }
}

/// UINTR 异步 Future
pub struct UintrFuture {
    token: UintrToken,
}

impl Future for UintrFuture {
    type Output = std::io::Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // 先检查是否有 pending 中断，避免时序问题
        let mut pending = self.token.inner.pending.lock().unwrap();
        if *pending {
            *pending = false;
            Poll::Ready(Ok(()))
        } else {
            // 没有 pending，保存 waker 并返回 Pending
            *self.token.inner.waker.lock().unwrap() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// 异步等待 UINTR 中断
pub async fn uintr(token: UintrToken) -> std::io::Result<()> {
    UintrFuture { token }.await
}

// 全局状态 - 使用两个文件描述符
static mut SERVER_UINTRFD: RawFd = -1;
static mut CLIENT_UINTRFD: RawFd = -1;
static mut CLIENT_UIPI_INDEX: c_int = -1;
static mut SERVER_UIPI_INDEX: c_int = -1;
static mut SERVER_SENT_COUNT: u32 = 0;
static mut CLIENT_SENT_COUNT: u32 = 0;

fn get_server_uintrfd() -> RawFd {
    unsafe { SERVER_UINTRFD }
}

fn set_server_uintrfd(fd: RawFd) {
    unsafe {
        SERVER_UINTRFD = fd;
    }
}

fn get_client_uintrfd() -> RawFd {
    unsafe { CLIENT_UINTRFD }
}

fn set_client_uintrfd(fd: RawFd) {
    unsafe {
        CLIENT_UINTRFD = fd;
    }
}

fn get_client_uipi_index() -> c_int {
    unsafe { CLIENT_UIPI_INDEX }
}

fn set_client_uipi_index(index: c_int) {
    unsafe {
        CLIENT_UIPI_INDEX = index;
    }
}

fn get_server_uipi_index() -> c_int {
    unsafe { SERVER_UIPI_INDEX }
}

fn set_server_uipi_index(index: c_int) {
    unsafe {
        SERVER_UIPI_INDEX = index;
    }
}

// Benchmark structure
#[derive(Clone)]
struct Benchmarks {
    total_start: Instant,
    single_start: Instant,
    minimum: Duration,
    maximum: Duration,
    sum: Duration,
    squared_sum: f64,
    count: usize,
    // 添加更多性能指标
    latencies: Vec<u64>, // 存储每个操作的延迟（纳秒）
    start_cpu_time: u64, // 开始时的CPU时间
    end_cpu_time: u64,   // 结束时的CPU时间
}

impl Benchmarks {
    fn new() -> Self {
        Benchmarks {
            total_start: Instant::now(),
            single_start: Instant::now(),
            minimum: Duration::from_secs(u64::MAX),
            maximum: Duration::from_nanos(0),
            sum: Duration::from_nanos(0),
            squared_sum: 0.0,
            count: 0,
            latencies: Vec::new(),
            start_cpu_time: 0,
            end_cpu_time: 0,
        }
    }

    // 重置总开始时间
    fn reset_total_start(&mut self) {
        self.total_start = Instant::now();
        // 记录开始时的CPU时间
        self.start_cpu_time = self.get_cpu_time();
    }

    // 开始测量单个操作
    fn start_operation(&mut self) {
        self.single_start = Instant::now();
    }

    // 结束测量单个操作并更新统计
    fn end_operation(&mut self) {
        let duration = self.single_start.elapsed();
        self.update(duration);
    }

    fn update(&mut self, duration: Duration) {
        let nanos = duration.as_nanos() as u64;
        self.latencies.push(nanos);
        self.minimum = self.minimum.min(duration);
        self.maximum = self.maximum.max(duration);
        self.sum += duration;
        self.squared_sum += nanos as f64 * nanos as f64;
        self.count += 1;
    }

    // 获取CPU时间（纳秒）
    fn get_cpu_time(&self) -> u64 {
        // 简化实现，使用Instant::now()的系统时间戳作为替代
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
    }

    // 计算分位数
    fn percentile(&self, p: f64) -> u64 {
        if self.latencies.is_empty() {
            return 0;
        }
        let mut sorted = self.latencies.clone();
        sorted.sort_unstable();
        let index = (sorted.len() as f64 * p / 100.0).floor() as usize;
        sorted[index.min(sorted.len() - 1)]
    }

    fn evaluate(&mut self, args: &Arguments) {
        let total_time = self.total_start.elapsed();
        let average = self.sum / (self.count as u32);

        let sigma = self.squared_sum / self.count as f64;
        let sigma = (sigma - (average.as_nanos() as f64).powi(2)).sqrt();

        let message_rate = (self.count as f64) / total_time.as_secs_f64();
        let message_rate_mb =
            (self.count as f64 * args.size as f64) / 1024.0 / 1024.0 / total_time.as_secs_f64();

        // 记录结束时的CPU时间
        self.end_cpu_time = self.get_cpu_time();
        let cpu_time_used = self.end_cpu_time.saturating_sub(self.start_cpu_time);
        let cpu_usage = (cpu_time_used as f64 / total_time.as_nanos() as f64) * 100.0;

        // 计算分位数
        let p50 = self.percentile(50.0);
        let p90 = self.percentile(90.0);
        let p99 = self.percentile(99.0);

        // 转换为微秒，确保小值不会显示为0
        let total_time_ms = total_time.as_secs_f64() * 1000.0;
        let average_us = average.as_nanos() as f64 / 1000.0;
        let minimum_us = self.minimum.as_nanos() as f64 / 1000.0;
        let maximum_us = self.maximum.as_nanos() as f64 / 1000.0;
        let sigma_us = sigma / 1000.0;
        let p50_us = p50 as f64 / 1000.0;
        let p90_us = p90 as f64 / 1000.0;
        let p99_us = p99 as f64 / 1000.0;

        println!("\n============ RESULTS ================");
        println!("Message size:       {}", args.size);
        println!("Message count:      {}", args.count);
        println!("Total duration:     {:.6} ms", total_time_ms);
        println!("Average duration:   {:.6} us", average_us);
        println!("Minimum duration:   {:.6} us", minimum_us);
        println!("Maximum duration:   {:.6} us", maximum_us);
        println!("Standard deviation: {:.6} us", sigma_us);
        println!("Latency P50:        {:.6} us", p50_us);
        println!("Latency P90:        {:.6} us", p90_us);
        println!("Latency P99:        {:.6} us", p99_us);
        println!("Message rate:       {:.0} msg/s", message_rate);
        println!("Message rate:       {:.3} MB/s", message_rate_mb);
        println!("CPU usage:          {:.2}%", cpu_usage);
        println!("=====================================");
    }
}

// 运行模式
#[derive(Clone, PartialEq)]
enum Mode {
    Server,
    Client,
    Both,
}

// Arguments structure
#[derive(Clone)]
struct Arguments {
    count: u32,
    size: usize,
    mode: Mode,
}

impl Arguments {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let mut count = 1000;
        let mut mode = Mode::Both;

        for arg in &args[1..] {
            if arg == "--server" {
                mode = Mode::Server;
            } else if arg == "--client" {
                mode = Mode::Client;
            } else if let Ok(c) = arg.parse() {
                count = c;
            }
        }

        Arguments { count, size: 1, mode }
    }
}

// 中断处理程序现在使用C语言版本实现

// 服务器等待UINTR
async fn server_uintrfd_wait() -> bool {
    let token = unsafe {
        SERVER_TOKEN_OBJ.as_ref().expect("SERVER_TOKEN_OBJ not initialized").clone()
    };
    
    match uintr(token).await {
        Ok(()) => {
            true
        },
        Err(_) => {
            false
        },
    }
}

// 客户端等待UINTR
async fn client_uintrfd_wait() -> bool {
    let token = unsafe {
        CLIENT_TOKEN_OBJ.as_ref().expect("CLIENT_TOKEN_OBJ not initialized").clone()
    };
    
    match uintr(token).await {
        Ok(()) => {
            true
        },
        Err(_) => {
            false
        },
    }
}

// 发送UINTR
fn uintrfd_notify(uipi_index: c_int) {
    if uipi_index < 0 {
        panic!("UIPI index not set");
    }

    unsafe {
        senduipi(uipi_index as u64);
    }
}

// 客户端设置
async fn setup_client() {
    // 初始化客户端 UintrToken
    unsafe {
        CLIENT_TOKEN_OBJ = Some(UintrToken::new("CLIENT"));
        CLIENT_INITIALIZED = true;
    }

    // 注册客户端中断处理程序
    match uintr_register_handler(client_ui_handler, 0) {
        Ok(res) => {
            println!("Client: Interrupt handler registered successfully: {}", res);
        }
        Err(err) => {
            panic!("Client interrupt handler register error: {}", err);
        }
    }

    // 创建客户端uintrfd文件描述符 - 使用向量1（CLIENT_TOKEN）
    let client_descriptor = match uintr_create_fd(1, 0) {
        Ok(fd) => fd,
        Err(err) => {
            panic!("Client interrupt vector registration error: {}", err);
        }
    };
    set_client_uintrfd(client_descriptor);
    println!(
        "Client: Created uintrfd with descriptor {} (vector 1)",
        client_descriptor
    );

    // 启用中断
    unsafe {
        stui();
    }
    println!("Client: Interrupts enabled");
}

// 服务端设置
async fn setup_server() {
    // 初始化服务器 UintrToken
    unsafe {
        SERVER_TOKEN_OBJ = Some(UintrToken::new("SERVER"));
        SERVER_INITIALIZED = true;
    }

    // 注册服务器中断处理程序
    match uintr_register_handler(server_ui_handler, 0) {
        Ok(res) => {
            println!("Server: Interrupt handler registered successfully: {}", res);
        }
        Err(err) => {
            panic!("Server interrupt handler register error: {}", err);
        }
    }

    // 创建服务器uintrfd文件描述符 - 使用向量0（SERVER_TOKEN）
    let server_descriptor = match uintr_create_fd(0, 0) {
        Ok(fd) => fd,
        Err(err) => {
            panic!("Server interrupt vector registration error: {}", err);
        }
    };
    set_server_uintrfd(server_descriptor);
    println!(
        "Server: Created uintrfd with descriptor {} (vector 0)",
        server_descriptor
    );

    // 启用中断
    unsafe {
        stui();
    }
    println!("Server: Interrupts enabled");
}

// 客户端通信函数
async fn client_communicate(
    args: Arguments,
    _client_ready: Arc<Notify>,
    _server_ready: Arc<Notify>,
    _test_completed: Arc<Notify>,
    test_done: Arc<AtomicBool>,
) {
    setup_client().await;

    println!("Client: Ready for communication");
    println!("Client: Starting communication for {} messages", args.count);

    // 连接到server的Unix Domain Socket
    let socket_path = "/tmp/uintr.sock";
    let mut server_socket = None;
    for _ in 0..1000 {
        match UnixStream::connect(socket_path) {
            Ok(s) => {
                server_socket = Some(s);
                println!("Client: Connected to server socket");
                break;
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    if let Some(s) = server_socket {
        // 发送client的文件描述符给server
        let client_fd = get_client_uintrfd();
        match send_fd(&s, client_fd) {
            Ok(_) => {
                println!("Client: Sent client file descriptor {}", client_fd);
            }
            Err(e) => {
                panic!("Client: Failed to send client fd: {}", e);
            }
        }

        // 接收server的文件描述符
        let server_fd = match recv_fd(&s) {
            Ok(fd) => {
                set_server_uintrfd(fd);
                println!("Client: Received server file descriptor {}", fd);
                fd
            }
            Err(e) => {
                panic!("Client: Failed to receive server fd: {}", e);
            }
        };

        // 注册发送者
        let _uipi_index = match uintr_register_sender(server_fd, 0) {
            Ok(index) => {
                set_client_uipi_index(index);
                println!("Client: Registered sender for server with UIPI index {}", index);
                index
            }
            Err(err) => {
                println!("Warning: Failed to register sender for server: {}", err);
                -1
            }
        };
    } else {
        panic!("Client: Timeout connecting to server socket");
    }

    println!("Client: Starting communication for {} messages", args.count);

    let mut message_count = 0;
    let uipi_index = get_client_uipi_index();
    while message_count < args.count && !test_done.load(std::sync::atomic::Ordering::Acquire) {
        // if message_count % 100 == 0 {
        //     println!("Client: Progress - {} / {}", message_count, args.count);
        // }
        
        // 等待来自服务端的中断
        if client_uintrfd_wait().await {
            // 发送响应中断
            uintrfd_notify(uipi_index);
            // if uipi_index >= 0 {
            //     if message_count % 100 == 0 {
            //         println!("Client: Sending response interrupt #{}", message_count);
            //     }
                
            //     unsafe {
            //         CLIENT_SENT_COUNT += 1;
            //     }
            //     message_count += 1;
            // } else {
            //     println!("Error: Client UIPI index not set");
            //     break;
            // }
        }
    }

    println!("Client: Communication complete");
}

// 服务端通信函数
async fn server_communicate(
    args: Arguments,
    _client_ready: Arc<Notify>,
    _server_ready: Arc<Notify>,
    _test_completed: Arc<Notify>,
    test_done: Arc<AtomicBool>,
) {
    setup_server().await;

    println!("Server: Ready for communication");
    println!("Server: Starting communication for {} messages", args.count);

    // 创建并监听Unix Domain Socket
    let socket_path = "/tmp/uintr.sock";
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)
        .expect("Server: Failed to bind socket");
    listener.set_nonblocking(true)
        .expect("Server: Failed to set nonblocking");
    println!("Server: Listening on {}", socket_path);

    // 等待客户端连接
    let mut client_socket = None;
    for _ in 0..1000 {
        match listener.accept() {
            Ok((s, _)) => {
                client_socket = Some(s);
                println!("Server: Client connected");
                break;
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    if let Some(s) = client_socket {
        // 接收client的文件描述符
        let client_fd = match recv_fd(&s) {
            Ok(fd) => {
                set_client_uintrfd(fd);
                println!("Server: Received client file descriptor {}", fd);
                fd
            }
            Err(e) => {
                panic!("Server: Failed to receive client fd: {}", e);
            }
        };

        // 发送server的文件描述符给client
        let server_fd = get_server_uintrfd();
        match send_fd(&s, server_fd) {
            Ok(_) => {
                println!("Server: Sent server file descriptor {}", server_fd);
            }
            Err(e) => {
                panic!("Server: Failed to send server fd: {}", e);
            }
        }

        // 注册发送者
        let _uipi_index = match uintr_register_sender(client_fd, 0) {
            Ok(index) => {
                set_server_uipi_index(index);
                println!("Server: Registered sender for client with UIPI index {}", index);
                index
            }
            Err(err) => {
                println!("Warning: Failed to register sender for client: {}", err);
                -1
            }
        };
    } else {
        panic!("Server: Timeout waiting for client connection");
    }

    // 设置基准测试
    let mut bench = Benchmarks::new();

    println!("Server: Starting communication for {} messages", args.count);

    // 重置总开始时间，确保从实际开始通信时计时
    bench.reset_total_start();
    let uipi_index = get_server_uipi_index();
    for i in 0..args.count {
        // if i % 100 == 0 {
        //     println!("Server: Progress - {} / {}", i, args.count);
        // }
        
        // 开始测量单个操作
        bench.start_operation();

        // 发送中断到客户端
        uintrfd_notify(uipi_index);
        
            // unsafe {
            //     SERVER_SENT_COUNT += 1;
            //     if i % 100 == 0 {
            //         println!("Server: Sent interrupt #{}", i);
            //     }
            // }

        // 等待响应
        server_uintrfd_wait().await;

        // 结束测量单个操作并更新统计
        bench.end_operation();
    }

    // 评估基准测试结果
    bench.evaluate(&args);

    // 标记测试已完成
    test_done.store(true, std::sync::atomic::Ordering::Release);
    println!("Server: Test completed");

    println!("Server: Communication complete");
}

// 主通信函数
async fn communicate(args: Arguments) {
    match args.mode {
        Mode::Server => {
            println!("Running as server");
            // 创建同步机制
            let client_ready = Arc::new(Notify::new());
            let server_ready = Arc::new(Notify::new());
            let test_completed = Arc::new(Notify::new());
            let test_done = Arc::new(AtomicBool::new(false));

            // 只运行服务端任务
            server_communicate(args, client_ready, server_ready, test_completed, test_done).await;
        }
        Mode::Client => {
            println!("Running as client");
            // 创建同步机制
            let client_ready = Arc::new(Notify::new());
            let server_ready = Arc::new(Notify::new());
            let test_completed = Arc::new(Notify::new());
            let test_done = Arc::new(AtomicBool::new(false));

            // 只运行客户端任务
            client_communicate(args, client_ready, server_ready, test_completed, test_done).await;
        }
        Mode::Both => {
            println!("Running as both server and client (same process)");
            // 创建一个同步机制，确保客户端先准备好
            let client_ready = Arc::new(Notify::new());
            let server_ready = Arc::new(Notify::new());
            let test_completed = Arc::new(Notify::new());
            let test_done = Arc::new(AtomicBool::new(false));

            let client_ready_clone = client_ready.clone();
            let server_ready_clone = server_ready.clone();
            let test_completed_clone = test_completed.clone();
            let test_done_clone = test_done.clone();

            // 创建客户端任务
            let client_args = args.clone();
            let client_task = tokio::task::spawn(async move {
                client_communicate(
                    client_args,
                    client_ready_clone,
                    server_ready_clone,
                    test_completed_clone,
                    test_done_clone,
                ).await;
            });

            // 创建服务端任务
            let server_args = args.clone();
            let server_task = tokio::task::spawn(async move {
                server_communicate(server_args, client_ready, server_ready, test_completed, test_done).await;
            });

            // 等待两个任务完成
            client_task.await.unwrap();
            server_task.await.unwrap();
        }
    }
}

fn main() {
    let args = Arguments::parse();

    // let rt = tokio::runtime::Builder::new_multi_thread()
    //     .worker_threads(1)
    //     .enable_all()
    //     .build()
    //     .unwrap();
let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    // .thread_keep_alive(std::time::Duration::MAX)  // 无限期保持线程活跃，只对spawn blocking任务生效
    .build()
    .unwrap();

    rt.block_on(async {
        // 创建定时唤醒任务，定期唤醒 worker 线程
        // tokio::spawn(async {
        //     let mut interval = tokio::time::interval(tokio::time::Duration::from_micros(5));
        //     loop {
        //         interval.tick().await;
        //         // println!("Wakeup worker thread");
        //     }
        // });
        // tokio::spawn(async {
        //     // tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        //     loop {
        //         // 做一点无害的运算，确保线程时刻在跑
        //         // 实践来看 2000 次循环 效果最好
        //         for _ in 0..2000 {
        //             std::hint::spin_loop();

        //         }
        //         // 可选：偶尔 yield 一下，避免饿死其他任务
        //         tokio::task::yield_now().await;
        //         // let mut i = I.lock().unwrap();
        //         // *i += 1;
        //         // println!("Yield worker thread {}", *i);
        //     }
        // });
        match args.mode {
            Mode::Server => {
                println!("Running as server");
                // 创建同步机制
                let client_ready = Arc::new(Notify::new());
                let server_ready = Arc::new(Notify::new());
                let test_completed = Arc::new(Notify::new());
                let test_done = Arc::new(AtomicBool::new(false));

                let server_task = tokio::spawn(server_communicate(
                    args.clone(),
                    client_ready,
                    server_ready,
                    test_completed,
                    test_done.clone(),
                ));

                server_task.await.unwrap();
            }
            Mode::Client => {
                println!("Running as client");
                // 创建同步机制
                let client_ready = Arc::new(Notify::new());
                let server_ready = Arc::new(Notify::new());
                let test_completed = Arc::new(Notify::new());
                let test_done = Arc::new(AtomicBool::new(false));

                let client_task = tokio::spawn(client_communicate(
                    args.clone(),
                    client_ready,
                    server_ready,
                    test_completed,
                    test_done.clone(),
                ));

                client_task.await.unwrap();
            }
            Mode::Both => {
                println!("Running as both server and client (same process)");
                // 创建同步机制
                let client_ready = Arc::new(Notify::new());
                let server_ready = Arc::new(Notify::new());
                let test_completed = Arc::new(Notify::new());
                let test_done = Arc::new(AtomicBool::new(false));

                let mut server_task = tokio::spawn(server_communicate(
                    args.clone(),
                    client_ready.clone(),
                    server_ready.clone(),
                    test_completed.clone(),
                    test_done.clone(),
                ));

                let mut client_task = tokio::spawn(client_communicate(
                    args.clone(),
                    client_ready,
                    server_ready,
                    test_completed,
                    test_done.clone(),
                ));

                tokio::select! {
                    _ = &mut server_task => {
                        test_done.store(true, std::sync::atomic::Ordering::Release);
                        let _ = client_task.abort();
                    }
                    _ = &mut client_task => {
                        test_done.store(true, std::sync::atomic::Ordering::Release);
                        let _ = server_task.abort();
                    }
                }
            }
        }

        // 清理临时文件
        let _ = std::fs::remove_file("/tmp/uintr.sock");
        let _ = std::fs::remove_file("/tmp/server_uintr_info");
        let _ = std::fs::remove_file("/tmp/client_uintr_info");
    });
}
