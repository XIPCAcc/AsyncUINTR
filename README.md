# UINTR-BI: User-Level Interrupt Benchmark for Bidirectional Communication

基于用户态中断（UINTR）的双向通信基准测试，用于测量服务器与客户端之间的通信性能。

## 功能特性

- **双向通信**：服务器和客户端使用用户态中断进行通信
- **性能指标**：详细的测量包括：
  - 平均、最小、最大延迟
  - 延迟分布（P50、P90、P99）
  - 标准差
  - CPU使用率
- **线程安全**：使用原子操作确保跨线程通信安全
- **独立处理程序**：服务器和客户端使用独立的中断处理程序
- **C-Rust互操作**：C语言中断处理程序可以调用Rust回调函数

## 模块说明

- `main.rs`：主程序，包含通信逻辑和基准测试
- `handler.c`：C语言中断处理程序
- `build.rs`：构建脚本，自动检测C代码变化

## 使用方法

```bash
cargo run -- <message_count>
```

### 参数说明

- `message_count`：服务器和客户端之间交换的消息数量（默认：1000）

### 示例

运行10条消息的测试：
```bash
cargo run -- 10
```

## 输出示例

```
Client: Interrupt handler registered successfully: 0
Client: Created uintrfd with descriptor 3
Server: Interrupt handler registered successfully: 0
Server: Created uintrfd with descriptor 4
Server: Registered sender for client with UIPI index 0
Server: Interrupts enabled
Client: Registered sender for server with UIPI index 1
Client: Interrupts enabled
Client: Ready for communication
Server: Client is ready, starting communication
Server: Ready for communication
Client: Server is ready, starting communication
Client: Starting communication for 100 messages
Server: Starting communication for 100 messages
Client: Received interrupt, vector=1
Rust callback: Client received interrupt, vector=1
Server: Received interrupt, vector=0
Rust callback: Server received interrupt, vector=0
...

============ RESULTS ================
Message size:       1
Message count:      100
Total duration:     0.097551 ms
Average duration:   0.897000 us
Minimum duration:   0.828000 us
Maximum duration:   1.297000 us
Standard deviation: 0.062508 us
Latency P50:        0.885000 us
Latency P90:        0.924000 us
Latency P99:        1.297000 us
Message rate:       1,025,026 msg/s
Message rate:       0.977 MB/s
CPU usage:          100.00%
=====================================
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
- Linux内核支持UINTR
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

##### 为什么不能在中断处理程序中调用printf？

中断处理程序中调用`printf`会导致编译错误，但是能在Rust回调函数`rust_interrupt_callback`调用`println!`（只是能编译过，不代表是安全的）

5. **实际代码示例**：
   ```c
   // 中断处理程序（C）- 只设置标志
   void server_ui_handler(...) {
       uintr_received[vector] = 1;  // 最小化操作
       rust_interrupt_callback("Server", vector);  // 可以调用，但实际处理在Rust中
   }
   ```

   ```rust
   // Rust回调函数 - 可以安全使用println!
   #[no_mangle]
   pub extern "C" fn rust_interrupt_callback(handler_name: &CStr, vector: u64) {
       let name = handler_name.to_str().unwrap();
       println!("Rust callback: {} received interrupt, vector={}", name, vector);
       // 可以执行任意复杂的逻辑
   }
   ```

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
