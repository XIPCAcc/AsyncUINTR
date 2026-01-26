#include <stdint.h>

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

// 服务器中断处理程序
void __attribute__ ((interrupt))
     __attribute__((target("general-regs-only", "inline-all-stringops")))
     server_ui_handler(struct UintrFrame *ui_frame,
	 	 unsigned long long vector) {

	 	 // The vector number is same as the token
	 	 uintr_received[vector] = 1;
}

// 客户端中断处理程序
void __attribute__ ((interrupt))
     __attribute__((target("general-regs-only", "inline-all-stringops")))
     client_ui_handler(struct UintrFrame *ui_frame,
	 	 unsigned long long vector) {

	 	 // The vector number is same as the token
	 	 uintr_received[vector] = 1;
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