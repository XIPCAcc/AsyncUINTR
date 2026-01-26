#![warn(unused)]
use libc::{c_int, c_long, syscall};
use std::thread;
use std::os::unix::io::RawFd;
use core::arch::asm;
use std::time::{Instant, Duration};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

// UINTR栈帧结构（必须与内核一致）
#[repr(C)]
pub struct UintrFrame {
    pub rip: u64,      // 中断返回地址
    pub rflags: u64,   // 标志寄存器
    pub rsp: u64,      // 栈指针
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
fn uintr_register_handler(handler: unsafe extern "C" fn(*mut UintrFrame, u64), flags: c_int) -> Result<c_int, String> {
    let result = unsafe { syscall(__NR_UINTR_REGISTER_HANDLER, handler, flags) as c_int };
    if result < 0 {
        Err(format!("uintr_register_handler failed: {}", std::io::Error::last_os_error()))
    } else {
        Ok(result)
    }
}

fn uintr_create_fd(vector: c_int, flags: c_int) -> Result<RawFd, String> {
    let result = unsafe { syscall(__NR_UINTR_CREATE_FD, vector, flags) as RawFd };
    if result < 0 {
        Err(format!("uintr_create_fd failed: {}", std::io::Error::last_os_error()))
    } else {
        Ok(result)
    }
}

fn uintr_register_sender(fd: RawFd, flags: c_int) -> Result<c_int, String> {
    let result = unsafe { syscall(__NR_UINTR_REGISTER_SENDER, fd, flags) as c_int };
    if result < 0 {
        Err(format!("uintr_register_sender failed: {}", std::io::Error::last_os_error()))
    } else {
        Ok(result)
    }
}

// 定义向量/令牌常量
const SERVER_TOKEN: u64 = 0;
const CLIENT_TOKEN: u64 = 1;

// 全局状态 - 使用两个文件描述符
static mut SERVER_UINTRFD: RawFd = -1;
static mut CLIENT_UINTRFD: RawFd = -1;
static mut CLIENT_UIPI_INDEX: c_int = -1;
static mut SERVER_UIPI_INDEX: c_int = -1;

fn get_server_uintrfd() -> RawFd {
    unsafe { SERVER_UINTRFD }
}

fn set_server_uintrfd(fd: RawFd) {
    unsafe { SERVER_UINTRFD = fd; }
}

fn get_client_uintrfd() -> RawFd {
    unsafe { CLIENT_UINTRFD }
}

fn set_client_uintrfd(fd: RawFd) {
    unsafe { CLIENT_UINTRFD = fd; }
}

fn get_client_uipi_index() -> c_int {
    unsafe { CLIENT_UIPI_INDEX }
}

fn set_client_uipi_index(index: c_int) {
    unsafe { CLIENT_UIPI_INDEX = index; }
}

fn get_server_uipi_index() -> c_int {
    unsafe { SERVER_UIPI_INDEX }
}

fn set_server_uipi_index(index: c_int) {
    unsafe { SERVER_UIPI_INDEX = index; }
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
        let message_rate_mb = (self.count as f64 * args.size as f64) / 1024.0 / 1024.0 / total_time.as_secs_f64();
        
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

// Arguments structure
#[derive(Clone)]
struct Arguments {
    count: u32,
    size: usize,
}

impl Arguments {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let mut count = 1000;

        for arg in &args[1..] {
            if let Ok(c) = arg.parse() {
                count = c;
            }
        }

        Arguments {
            count,
            size: 1,
        }
    }
}

// 中断处理程序现在使用C语言版本实现

// 服务器等待UINTR，支持超时
fn server_uintrfd_wait(timeout_ms: Option<u64>) -> bool {
    // 轮询直到接收到中断或超时
    let start = Instant::now();
    let mut spin_count = 0;
    while unsafe { get_server_uintr_received() } == 0 {
        // 检查是否超时
        if let Some(timeout) = timeout_ms {
            if start.elapsed() > Duration::from_millis(timeout) {
                return false;
            }
        }
        
        // 优化CPU使用率：先自旋几次，然后再yield
        spin_count += 1;
        if spin_count > 100 {
            std::thread::yield_now();
            spin_count = 0;
        } else {
            // 短暂的空操作，减少CPU使用率
            unsafe { asm!("pause", options(nostack, nomem)); }
        }
    }

    // 重置标志
    unsafe { set_server_uintr_received(0); }
    true
}

// 客户端等待UINTR，支持超时
fn client_uintrfd_wait(timeout_ms: Option<u64>) -> bool {
    // 轮询直到接收到中断或超时
    let start = Instant::now();
    let mut spin_count = 0;
    while unsafe { get_client_uintr_received() } == 0 {
        // 检查是否超时
        if let Some(timeout) = timeout_ms {
            if start.elapsed() > Duration::from_millis(timeout) {
                return false;
            }
        }
        
        // 优化CPU使用率：先自旋几次，然后再yield
        spin_count += 1;
        if spin_count > 100 {
            std::thread::yield_now();
            spin_count = 0;
        } else {
            // 短暂的空操作，减少CPU使用率
            unsafe { asm!("pause", options(nostack, nomem)); }
        }
    }

    // 重置标志
    unsafe { set_client_uintr_received(0); }
    true
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
fn setup_client() {
    // 注册客户端中断处理程序
    match uintr_register_handler(client_ui_handler, 0) {
        Ok(res) => {
            println!("Client: Interrupt handler registered successfully: {}", res);
        }
        Err(err) => {
            panic!("Client interrupt handler register error: {}", err);
        }
    }

    // 创建客户端uintrfd文件描述符
    let client_descriptor = match uintr_create_fd(CLIENT_TOKEN as c_int, 0) {
        Ok(fd) => fd,
        Err(err) => {
            panic!("Client interrupt vector registration error: {}", err);
        }
    };
    set_client_uintrfd(client_descriptor);
    println!("Client: Created uintrfd with descriptor {}", client_descriptor);

    // 等待服务端设置其FD
    while get_server_uintrfd() < 0 {
        std::thread::sleep(Duration::from_micros(10));
    }

    // 注册发送者
    let uipi_index = match uintr_register_sender(get_server_uintrfd(), 0) {
        Ok(index) => index,
        Err(err) => {
            unsafe {
                libc::close(client_descriptor);
            }
            panic!("Sender register error for server: {}", err);
        }
    };

    set_client_uipi_index(uipi_index);
    println!("Client: Registered sender for server with UIPI index {}", uipi_index);

    // 启用中断
    unsafe {
        stui();
    }
    println!("Client: Interrupts enabled");
}

// 服务端设置
fn setup_server() {
    // 注册服务器中断处理程序
    match uintr_register_handler(server_ui_handler, 0) {
        Ok(res) => {
            println!("Server: Interrupt handler registered successfully: {}", res);
        }
        Err(err) => {
            panic!("Server interrupt handler register error: {}", err);
        }
    }

    // 创建服务器uintrfd文件描述符
    let server_descriptor = match uintr_create_fd(SERVER_TOKEN as c_int, 0) {
        Ok(fd) => fd,
        Err(err) => {
            panic!("Server interrupt vector registration error: {}", err);
        }
    };
    set_server_uintrfd(server_descriptor);
    println!("Server: Created uintrfd with descriptor {}", server_descriptor);

    // 等待客户端设置其FD
    while get_client_uintrfd() < 0 {
        std::thread::sleep(Duration::from_micros(10));
    }

    // 注册发送者
    let uipi_index = match uintr_register_sender(get_client_uintrfd(), 0) {
        Ok(index) => index,
        Err(err) => {
            unsafe {
                libc::close(server_descriptor);
            }
            panic!("Sender register error for client: {}", err);
        }
    };

    set_server_uipi_index(uipi_index);
    println!("Server: Registered sender for client with UIPI index {}", uipi_index);

    // 启用中断
    unsafe {
        stui();
    }
    println!("Server: Interrupts enabled");
}

// 客户端通信函数
fn client_communicate(args: Arguments, client_ready: std::sync::Arc<std::sync::atomic::AtomicBool>, server_ready: std::sync::Arc<std::sync::atomic::AtomicBool>, test_completed: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    setup_client();

    // 标记客户端已准备好
    client_ready.store(true, std::sync::atomic::Ordering::Release);
    println!("Client: Ready for communication");

    // 等待服务器准备好
    while !server_ready.load(std::sync::atomic::Ordering::Acquire) {
        std::thread::yield_now();
    }
    println!("Client: Server is ready, starting communication");

    println!("Client: Starting communication for {} messages", args.count);

    let mut message_count = 0;
    while message_count < args.count && !test_completed.load(std::sync::atomic::Ordering::Acquire) {
        // 等待来自服务端的中断，设置500ms超时
        if client_uintrfd_wait(Some(500)) {
            // 发送响应中断
            uintrfd_notify(get_client_uipi_index());
            message_count += 1;
        }
    }

    println!("Client: Communication complete");
}

// 服务端通信函数
fn server_communicate(args: Arguments, client_ready: std::sync::Arc<std::sync::atomic::AtomicBool>, server_ready: std::sync::Arc<std::sync::atomic::AtomicBool>, test_completed: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    setup_server();

    // 等待客户端准备好
    while !client_ready.load(std::sync::atomic::Ordering::Acquire) {
        std::thread::yield_now();
    }
    println!("Server: Client is ready, starting communication");

    // 标记服务器已准备好
    server_ready.store(true, std::sync::atomic::Ordering::Release);
    println!("Server: Ready for communication");

    // 设置基准测试
    let mut bench = Benchmarks::new();

    println!("Server: Starting communication for {} messages", args.count);

    // 重置总开始时间，确保从实际开始通信时计时
    bench.reset_total_start();

    for _ in 0..args.count {
        // 开始测量单个操作
        bench.start_operation();

        // 发送中断到客户端
        uintrfd_notify(get_server_uipi_index());

        // 等待响应，设置500ms超时
        server_uintrfd_wait(Some(500));

        // 结束测量单个操作并更新统计
        bench.end_operation();
    }

    // 评估基准测试结果
    bench.evaluate(&args);

    // 标记测试已完成
    test_completed.store(true, std::sync::atomic::Ordering::Release);
    println!("Server: Test completed");

    println!("Server: Communication complete");
}

// 主通信函数
fn communicate(args: Arguments) {
    // 创建一个同步机制，确保客户端先准备好
    let client_ready = Arc::new(AtomicBool::new(false));
    let server_ready = Arc::new(AtomicBool::new(false));
    let test_completed = Arc::new(AtomicBool::new(false));
    
    let client_ready_clone = client_ready.clone();
    let server_ready_clone = server_ready.clone();
    let test_completed_clone = test_completed.clone();
    
    // 创建客户端线程
    let client_args = args.clone();
    let client_thread = thread::spawn(move || {
        client_communicate(client_args, client_ready_clone, server_ready_clone, test_completed_clone);
    });
    
    // 创建服务端线程
    let server_args = args.clone();
    let server_thread = thread::spawn(move || {
        server_communicate(server_args, client_ready, server_ready, test_completed);
    });
    
    // 等待两个线程完成
    client_thread.join().unwrap();
    server_thread.join().unwrap();
}

fn main() {
    let args = Arguments::parse();

    // 运行通信测试
    communicate(args);

    // 清理资源
    let server_fd = get_server_uintrfd();
    let client_fd = get_client_uintrfd();

    unsafe {
        if server_fd >= 0 {
            libc::close(server_fd);
        }
        if client_fd >= 0 {
            libc::close(client_fd);
        }
    }
}
