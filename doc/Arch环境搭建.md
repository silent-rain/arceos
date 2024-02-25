# Arch 环境搭建

## QEMU 模拟器安装

```shell
sudo pacman -S qemu-system-riscv

qemu-system-riscv64 --version
# qemu-riscv64 --version
```

## 其他工具安装

```shell
# 用于调用 Rust 工具链中的 LLVM 工具的 Cargo 子命令, 为了使用 objdump、objcopy 工具
cargo install cargo-binutils

# 用于支持 Clang 的开发库
sudo pacman -S clang

# 用于交叉编译目标为 musl Linux 的工具链
sudo pacman -S aarch64-linux-musl-cross-bin
```

## 相关文档

- [ArceOS Tutorial Book](https://rcore-os.cn/arceos-tutorial-book/index.html)
- [rCore-Tutorial-Book 第三版](https://rcore-os.cn/rCore-Tutorial-Book-v3/index.html)
