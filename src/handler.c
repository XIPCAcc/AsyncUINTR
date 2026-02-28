#include <stdint.h>
#include <stdio.h>
#include <unistd.h>
#include <string.h>
#include <sys/syscall.h>
#include <stdbool.h>

// 系统调用号
#define __NR_uintr_wait 476

// UINTR栈帧结构（必须与内核一致）
struct UintrFrame {
    uint64_t rip;      // 中断返回地址
    uint64_t rflags;   // 标志寄存器
    uint64_t rsp;      // 栈指针
};

// 定义向量/令牌常量
#define SERVER_TOKEN 0
#define CLIENT_TOKEN 1

// 全局状态 - 使用两个文件描述符
volatile unsigned long uintr_received[2] = {0, 0};

// 声明Rust回调函数
extern void rust_interrupt_callback(const char *handler_name, unsigned long long vector);

// 辅助函数：使用write系统调用打印
static void print_interrupt(const char *prefix, unsigned long long vector) {
    char buffer[128];
    int len = snprintf(buffer, sizeof(buffer), "%s: Received interrupt, vector=%llu\n", prefix, vector);
    ssize_t result = write(STDOUT_FILENO, buffer, len);
    (void)result;
}

// 服务器中断处理程序
void __attribute__ ((interrupt))
     __attribute__((target("general-regs-only", "inline-all-stringops")))
     server_ui_handler(struct UintrFrame *_ui_frame __attribute__((unused)),
	 unsigned long long vector) {

	 // The vector number is same as token
	 uintr_received[vector] = 1;
	 // 调用Rust回调函数（Rust回调函数会处理打印）
	 rust_interrupt_callback("Server", vector);
}

// 客户端中断处理程序
void __attribute__ ((interrupt))
     __attribute__((target("general-regs-only", "inline-all-stringops")))
     client_ui_handler(struct UintrFrame *_ui_frame __attribute__((unused)),
	 unsigned long long vector) {

	 // The vector number is same as token
	 uintr_received[vector] = 1;
	 // 调用Rust回调函数（Rust回调函数会处理打印）
	 rust_interrupt_callback("Client", vector);
}

// 获取服务器中断标志
int get_server_uintr_received(void) {
    return uintr_received[SERVER_TOKEN];
}

// 获取客户端中断标志
int get_client_uintr_received(void) {
    return uintr_received[CLIENT_TOKEN];
}

// 设置服务器中断标志
void set_server_uintr_received(int value) {
    uintr_received[SERVER_TOKEN] = value;
}

// 设置客户端中断标志
void set_client_uintr_received(int value) {
    uintr_received[CLIENT_TOKEN] = value;
}

// uintr_wait 系统调用包装函数
bool uintr_wait(int flags) {
    long result = syscall(__NR_uintr_wait, flags);
    return result == 0;
}