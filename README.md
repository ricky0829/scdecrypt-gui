# scdecrypt-gui

一个用于解密 Syncthing 加密数据的小工具。无需再重新部署 Syncthing、配对设备、拉取整个 Folder 进行解密。

## 功能

- 文件夹解密：解密整个加密文件夹
- 文件解密：解密指定的加密文件
- 支持手动替换 syncthing 程序，也可以通过内置更新功能获取最新版

## 使用方法

- 从 Release 下载（目前仅提供 Windows X64，其他架构可自行编译）
- 程序会自动检测同目录下的 syncthing 程序，若不存在，可通过内置更新器下载
- 准备 Folder ID（若忘记，可在 .stfloder 目录中查询）、加密密码
- 选择要解密的文件夹或文件，输入 Folder ID 和密码
- 选择要输出的路径
- 点击解密，Enjoy！

## 运行截图


## 技术栈

| 层 | 技术 |
|----|------|
| 桌面框架 | [Tauri 2](https://tauri.app/) |
| 后端 | Rust |
| 前端 | 原生 TypeScript + Vite（无框架，轻量） |
| 解密核心 | Syncthing 官方 `decrypt` 子命令 |

## 目录结构

```
scdecrypt-gui/
├── build.bat               # 编译脚本
├── index.html              # 页面结构
├── package.json
├── package-lock.json
├── tsconfig.json           # TypeScript 配置
├── vite.config.ts          # Vite 配置
├── src/
│   ├── main.ts             # 前端交互
│   └── styles.css          # 样式
└── src-tauri/
    ├── Cargo.toml          # Rust 依赖清单
    ├── Cargo.lock
    ├── build.rs            # Tauri 构建脚本
    ├── tauri.conf.json     # 窗口与应用配置
    ├── capabilities/
    │   └── default.json    # 权限声明
    ├── icons/              # 应用图标
    └── src/
        ├── main.rs         # 程序入口
        └── lib.rs          # Rust 后端
```



## 编译

### 环境要求

- [Node.js](https://nodejs.org/)
- [Rust](https://rustup.rs/)（MSVC 工具链）+ Visual Studio Build Tools（C++ 负载）

### 编译脚本

直接运行 `build.bat` 即可，输出路径：`src-tauri\target\release\scdecrypt-gui.exe`