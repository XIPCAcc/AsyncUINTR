# UINTR-BI: User-Level Interrupt Benchmark for Bidirectional Communication

基于用户态中断（UINTR）的双向通信基准测试，用于测量服务器与客户端之间的通信性能。

## 功能特性

- **双向通信**：服务器和客户端使用用户态中断进行通信
- **独立进程**：服务器和客户端作为独立进程运行，更真实地模拟实际场景
- **性能指标**：详细的测量包括：
  - 平均、最小、最大延迟
  - 延迟分布（P50、P90、P99）
  - 标准差
  - CPU使用率
- **线程安全**：使用原子操作确保跨线程通信安全
- **独立处理程序**：服务器和客户端使用独立的中断处理程序
- **C-Rust互操作**：C语言中断处理程序可以调用Rust回调函数
- **管道+Watch Channel**：使用管道和Tokio的watch channel实现可靠的异步中断处理

## 模块说明

- `main.rs`：主程序，包含通信逻辑和基准测试
- `handler.c`：C语言中断处理程序
- `build.rs`：构建脚本，自动检测C代码变化

## 使用方法

### 服务器模式

```bash
cargo run -- --server
```

### 客户端模式

```bash
cargo run -- --client
```

### 参数说明

- `--server`：以服务器模式运行
- `--client`：以客户端模式运行
- `message_count`：服务器和客户端之间交换的消息数量（默认：1000）

### 示例

运行10条消息的测试：

服务器：
```bash
cargo run -- --server 10
```

客户端：
```bash
cargo run -- --client 10
```

## 输出示例

### 服务器输出

```
Running as server
Server: Interrupt handler registered successfully: 0
Server: Created uintrfd with descriptor 11 (vector 0)
Server: Interrupts enabled
Server: Ready for communication
Server: Starting communication for 1000 messages
Server: Listening on /tmp/uintr.sock
Server: Client connected
Server: Received client file descriptor 14
Server: Sent server file descriptor 11
Server: Registered sender for client with UIPI index 0
Server: Starting communication for 1000 messages
Interrupt callback: SERVER interrupt received
...

============ RESULTS ================
Message size:       1
Message count:      1000
Total duration:     172.927719 ms
Average duration:   172.777000 us
Minimum duration:   38.535000 us
Maximum duration:   16019.181000 us
Standard deviation: 761.919443 us
Latency P50:        97.213000 us
Latency P90:        128.734000 us
Latency P99:        2089.781000 us
Message rate:       5783 msg/s
Message rate:       0.006 MB/s
CPU usage:          100.00%
=====================================
Server: Test completed
Server: Communication complete
```

### 客户端输出

```
Running as client
Client: Interrupt handler registered successfully: 0
Client: Created uintrfd with descriptor 11 (vector 1)
Client: Interrupts enabled
Client: Ready for communication
Client: Starting communication for 1000 messages
Client: Connected to server socket
Client: Sent client file descriptor 11
Interrupt callback: CLIENT interrupt received
Client: Received server file descriptor 13
Client: Registered sender for server with UIPI index 0
Client: Starting communication for 1000 messages
Interrupt callback: CLIENT interrupt received
...
Client: Communication complete
```

## 性能指标说明

- **Message size**：每条消息的大小（字节）
- **Message count**：总消息数量
- **Total duration**：测试总时间（毫秒）
- **Average duration**：平均每条消息的处理时间（微秒）
- **Minimum duration**：最快的消息处理时间
- **Maximum duration**：最慢的消息处理时间
- **Standard deviation**：延迟的标准差，衡量变异性
- **Latency P50/P90/P99**：50%、90%、99%分位延迟
- **Message rate**：消息速率（msg/s）
- **Message rate (MB/s)**：数据吞吐量（MB/s）
- **CPU usage**：CPU使用率

## 系统要求

- Rust 1.60+
- Linux内核支持UINTR (CONFIG_X86_USER_INTERRUPTS=y)
- 支持用户态中断的硬件
- GCC编译器（用于C代码编译）

## 构建

```bash
cargo build
```

优化构建：
```bash
cargo build --release
```

## 项目结构

```
src/
├── main.rs      # 主程序和通信逻辑
└── handler.c    # C语言中断处理程序

build.rs            # 构建脚本，自动检测C代码变化
Cargo.toml           # 项目配置
```

## 技术细节

### 中断处理流程

1. **中断回调**：C语言中断处理程序调用Rust回调函数
2. **管道写入**：回调函数写入管道并设置pending标志
3. **事件广播**：管道读取任务读取数据并通过watch channel广播事件
4. **异步等待**：使用watch receiver等待事件并处理

### 管道+Watch Channel实现

参考Tokio的signal处理机制，使用以下组件：

- **管道**：用于内核级别的唤醒，确保可靠地唤醒epoll
- **AtomicBool**：用于标记中断是否到达
- **Watch Channel**：用于在异步任务之间广播事件
- **计数器**：确保每次都能触发changed事件

### 中断处理程序属性

C语言中断处理程序使用以下GCC属性：

- `interrupt`：标记为中断处理程序，自动保存/恢复寄存器
- `general-regs-only`：限制只使用通用寄存器，不使用SIMD寄存器
- `inline-all-stringops`：内联字符串操作函数，减少函数调用开销

##### `__attribute__ ((interrupt))`

这是GCC的中断处理程序属性，它会：

1. **自动生成函数序言和结语**：自动保存和恢复被调用者保存的寄存器
2. **使用`iret`指令返回**：而不是普通的`ret`指令，确保正确返回到中断前的状态
3. **禁用编译器优化**：防止编译器对中断处理程序进行不安全的优化
4. **设置正确的栈对齐**：确保栈指针在调用时16字节对齐

##### `__attribute__((target("general-regs-only", "inline-all-stringops")))`

这个属性组合有两个重要目的：

**`general-regs-only`**：
- 限制编译器只使用通用寄存器（rax, rbx, rcx, rdx, rsi, rdi, rbp, rsp, r8-r15）
- 禁止使用SIMD/XMM寄存器（xmm0-xmm15）
- 原因：中断处理程序不应该修改SIMD寄存器，因为：
  - SIMD寄存器保存/恢复开销大
  - 中断可能在SIMD指令执行过程中发生，保存状态复杂
  - 避免影响被中断代码的SIMD状态

**`inline-all-stringops`**：
- 强制内联字符串操作函数（如`memcpy`, `memset`, `strlen`等）
- 避免函数调用开销
- 减少栈使用
- 提高中断处理程序的执行速度

### C-Rust互操作

项目展示了C语言中断处理程序如何调用Rust函数：

1. C代码声明外部Rust函数：`extern void rust_interrupt_callback(...)`
2. Rust定义可被C调用的函数：`#[unsafe(no_mangle)] pub extern "C" fn ...`
3. 通过标准C ABI进行通信

### 自动重新编译

`build.rs`中配置了：
```rust
println!("cargo:rerun-if-changed=src/handler.c");
```

当`src/handler.c`文件被修改时，Cargo会自动重新编译C代码。