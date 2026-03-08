# UINTR-BI: User-Level Interrupt Benchmark for Bidirectional Communication

## 使用方法

### 启动服务器

```bash
cargo run -- --server <message_count>
```

### 启动客户端

```bash
cargo run -- --client <message_count>
```

### 参数说明

- `--server`：以服务器模式运行
- `--client`：以客户端模式运行
- `message_count`：交换的消息数量（默认：1000）

### 示例

在两个终端中分别运行：

```bash
# 终端1 - 启动服务器
cargo run -- --server 10

# 终端2 - 启动客户端
cargo run -- --client 10
```

## 输出示例

```
Server: Interrupt handler registered successfully: 0
Server: Created uintrfd with descriptor 9 (vector 0)
Server: Interrupts enabled
Server: Ready for communication
Server: Starting communication for 10 messages
Server: Listening on /tmp/uintr.sock

Client: Interrupt handler registered successfully: 0
Client: Created uintrfd with descriptor 9 (vector 1)
Client: Interrupts enabled
Client: Ready for communication
Client: Starting communication for 10 messages
Client: Connected to server socket
Client: Sent client file descriptor 9
Client: Received server file descriptor 11
Client: Registered sender for server with UIPI index 0

============ RESULTS ================
Message size:       1
Message count:      10
Total duration:     0.145707 ms
Average duration:   14.145000 us
Minimum duration:   2.640000 us
Maximum duration:   17.467000 us
Standard deviation: 3.900149 us
Latency P50:        15.111000 us
Latency P90:        17.467000 us
Latency P99:        17.467000 us
Message rate:       68631 msg/s
Message rate:       0.065 MB/s
CPU usage:          100.28%
=====================================
```
