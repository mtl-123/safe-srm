# srm - 安全文件删除工具（带审计追踪与回收站功能）
基于Rust开发的`rm`安全替代工具，提供可恢复删除、审计日志、回收站管理、定期自动清理能力，支持按用户独立配置且不影响系统原生`rm`，适配TB级文件操作性能，编译产物支持UPX极致压缩

## 目录
- [概述](#概述)
- [核心特性](#核心特性)
- [开发与运行环境](#开发与运行环境)
  - [开发环境（精准匹配配置）](#开发环境精准匹配配置)
  - [运行环境](#运行环境)
- [安装步骤](#安装步骤)
  - [源码编译（含UPX压缩，推荐）](#源码编译含upx压缩推荐)
  - [二进制直接使用](#二进制直接使用)
- [核心命令使用指南](#核心命令使用指南)
  - [delete（删除文件/目录）](#delete删除文件目录)
  - [restore（恢复回收站项）](#restore恢复回收站项)
  - [list（列出回收站内容）](#list列出回收站内容)
  - [clean（清理回收站）](#clean清理回收站)
  - [empty（永久清空回收站）](#empty永久清空回收站)
  - [全局帮助](#全局帮助)
- [自动清理与Systemd服务配置](#自动清理与systemd服务配置)
- [安全替代原生rm（按用户独立生效）](#安全替代原生rm按用户独立生效)
  - [方式1：临时生效（当前终端）](#方式1临时生效当前终端)
  - [方式2：永久生效（仅当前用户，推荐）](#方式2永久生效仅当前用户推荐)
  - [关键说明](#关键说明)
- [配置说明](#配置说明)
  - [核心配置设计](#核心配置设计)
  - [自定义配置（源码修改）](#自定义配置源码修改)
  - [核心数据目录结构](#核心数据目录结构)
- [性能优化特性](#性能优化特性)
- [日志与审计](#日志与审计)
  - [日志核心特点](#日志核心特点)
  - [日志存储路径](#日志存储路径)
  - [日志内容示例](#日志内容示例)
  - [日志查看与解析](#日志查看与解析)
- [安全防护机制](#安全防护机制)
- [常见问题](#常见问题)
- [SRM 终端自动补全（Bash/Zsh）](#srm-终端自动补全bashzsh)
  - [Bash 补全脚本](#bash-补全脚本)
  - [Zsh 补全脚本](#zsh-补全脚本)
- [Zsh 原生补全插件：_srm 脚本](#zsh-原生补全插件_srm-脚本)
  - [_srm 补全脚本（核心文件）](#_srm-补全脚本核心文件)
  - [插件生效步骤](#插件生效步骤)
  - [核心特性](#核心特性-1)
  - [测试验证](#测试验证)

## 概述
`srm`（Safe RM）是一款面向Linux系统的安全文件删除工具，核心解决原生`rm`**删除不可恢复、无操作记录、无安全校验**的痛点。工具将待删除文件/目录移动至专属回收站，生成唯一短ID用于快速恢复，记录全量JSON格式审计日志，并支持按过期时间自动清理、手动恢复/永久删除，同时针对大文件/大目录做了极致性能优化，适配企业级TB-scale操作场景。

所有核心数据（回收站、元数据、日志）均存储在`srm`可执行文件同级的`.srm`目录中，实现**按用户/按安装目录隔离**，不修改系统全局配置，完全不影响原生`rm`命令的使用。编译产物支持UPX压缩，大幅减小二进制体积，便于分发和部署。

## 核心特性
结合源码实现的全维度功能，兼顾安全、性能、易用性：
1. **可恢复删除机制**：删除文件并非直接销毁，而是移动至专属回收站，通过短ID可快速恢复，避免误删损失
2. **唯一短ID标识**：每个删除项生成6位带类型前缀的短ID（文件`f_`/目录`d_`/软链`l_`），支持短ID快速恢复/查询
3. **全量审计日志**：记录所有操作（删除/恢复/清理/空回收站），包含毫秒级时间戳、操作元数据，日志自动轮转（30天保留）
4. **完善回收站管理**：支持列出回收站（含过期状态/大小/过期时间）、恢复指定项、清理过期项、永久清空回收站
5. **TB级性能优化**：同文件系统即时重命名、跨文件系统CoW写时复制、大文件mmap分块传输，支持实时进度追踪
6. **UPX压缩支持**：编译产物可通过UPX极致压缩，二进制体积减小60%+，不损失执行性能
7. **系统路径保护**：默认禁止删除`/bin`/`/etc`/`/usr`等8个核心系统路径，防止误删导致系统崩溃
8. **磁盘空间校验**：删除前检查目标文件系统可用空间，单文件最大占用可用空间80%，防止磁盘占满
9. **原子化元数据**：删除项元数据（原路径/权限/UID/GID等）采用原子化写入，防止进程崩溃导致数据损坏
10. **中断安全回滚**：支持Ctrl+C中断操作，正在执行的删除任务会自动回滚，避免文件丢失/损坏
11. **严格权限隔离**：回收站、日志、元数据目录/文件均设置`0700/0600`权限，仅当前用户可访问
12. **跨Linux兼容**：基于Rust跨平台特性，无需修改代码即可在主流Linux发行版运行

## 开发与运行环境
### 开发环境（精准匹配配置）
- 操作系统：Debian 13 Trixie Desktop
- 开发语言：Rust（2021 Edition）
- 构建工具：Cargo 1.93.0 (083ac5135 2025-12-15)
- 开发依赖组件（Debian 13）：
  - build-essential：基础编译工具链
  - libssl-dev：SSL/TSL依赖（日志序列化/解析）
  - pkg-config：系统依赖管理工具
  - rustup：Rust版本管理工具
  - systemd-dev：Systemd服务开发依赖（可选）
  - git：代码版本控制工具（可选）
  - upx：二进制文件压缩工具（编译产物优化用）
- 项目核心文件：`main.rs`、`Cargo.toml`、`Cargo.lock`

### 运行环境
- 操作系统：Linux（内核≥3.10，支持`fallocate`/`ioctl FICLONE`）
- 支持的Linux发行版：
  1. Debian 11/12/13（Bullseye/Bookworm/Trixie）
  2. Ubuntu 20.04/22.04/24.04 LTS
  3. CentOS Stream 8/9、RHEL 8/9
  4. Fedora 38/39/40
  5. Arch Linux/Manjaro（滚动更新版）
  6. openSUSE Leap 15.5/Tumbleweed
- 系统依赖：
  - libc6 ≥2.28：系统基础C库
  - systemd ≥240：（可选，用于自动清理服务部署）
- 硬件要求：无特殊要求，磁盘剩余空间≥回收站所需空间（建议≥1GB）

## 安装步骤
### 源码编译（含UPX压缩，推荐）
编译后自动生成优化版二进制，通过UPX压缩减小体积，步骤如下：
1. 进入项目目录（核心文件：`main.rs`、`Cargo.toml`、`Cargo.lock`）
   ```bash
   cd /path/to/your/srm
   ```
2. 安装开发依赖（Debian 13 环境）
   ```bash
   sudo apt update && sudo apt install -y build-essential libssl-dev pkg-config rustup git upx jq
   ```
3. 初始化Rust环境（若未安装）
   ```bash
   rustup default stable
   source $HOME/.cargo/env  # 加载Rust环境变量
   ```
4. 生产环境编译（开启极致优化）
   ```bash
   cargo build --release  # 开启LTO/代码优化/符号剥离
   ```
5. UPX二进制压缩（减小体积60%+，不影响性能）
   ```bash
   upx --best --lzma target/release/srm  # --best：最高压缩比，--lzma：LZMA算法
   ```
6. 全局安装（所有用户可执行）
   ```bash
   sudo cp target/release/srm /usr/local/bin/
   sudo chmod +x /usr/local/bin/srm
   ```
7. 验证安装成功（自动创建`.srm`核心目录）
   ```bash
   srm --version
   # 输出：srm 1.2.1 (Meitao Lin <mtl>) 即为成功
   ```

### 二进制直接使用
若已获取编译并压缩后的`srm`二进制文件，直接部署即可：
```bash
# 复制到系统可执行目录
sudo cp /path/to/compressed/srm /usr/local/bin/
sudo chmod +x /usr/local/bin/srm
# 验证
srm --version
```

## 核心命令使用指南
所有子命令支持**别名**（如`delete`/`del`、`restore`/`res`），简化日常使用；核心数据目录：`srm`可执行文件同级的`.srm/`（含`trash/`、`meta/`、`srm.log`）。

### delete（删除文件/目录）
#### 用法
将文件/目录移动至回收站，生成唯一短ID，默认7天后过期，支持自定义过期时间。
```bash
srm delete [OPTIONS] <PATHS>...
# 别名：srm del（推荐，更简洁）
```

#### 参数
| 参数            | 简写 | 类型 | 说明                                        | 默认值 |
| --------------- | ---- | ---- | ------------------------------------------- | ------ |
| `--expire-days` | `-d` | 整数 | 自定义文件过期天数，过期后可自动清理        | 7      |
| `--force`       | `-f` | 布尔 | 强制删除：允许删除系统保护路径/含`..`的路径 | 禁用   |
| `--help`        | `-h` | -    | 查看该命令详细帮助                          | -      |

#### 示例
```bash
# 基础删除：单个文件，默认7天过期
srm del test.txt
# 批量删除：多个文件/目录，自定义15天过期
srm del -d 15 document.pdf /data/temp_dir/
# 强制删除：覆盖系统路径保护（谨慎使用）
srm del -f /usr/local/custom_temp_file
# 查看帮助
srm del --help
```

#### 执行结果
```
✅ test.txt → 🆔 f_a3b4c5 [1.2 MB]
```

### restore（恢复回收站项）
#### 用法
通过**短ID**或**回收站全ID**恢复指定项，支持恢复到原路径或自定义路径，可覆盖已存在文件。
```bash
srm restore [OPTIONS] <NAMES>...
# 别名：srm res（推荐）
```

#### 参数
| 参数       | 简写 | 类型 | 说明                              | 默认值 |
| ---------- | ---- | ---- | --------------------------------- | ------ |
| `--force`  | `-f` | 布尔 | 强制覆盖目标路径已存在的文件/目录 | 禁用   |
| `--target` | `-t` | 路径 | 自定义恢复路径，默认恢复到原路径  | 原路径 |
| `--help`   | `-h` | -    | 查看该命令详细帮助                | -      |

#### 示例
```bash
# 基础恢复：通过短ID恢复到原路径
srm res f_a3b4c5
# 批量恢复：多个项，强制覆盖已存在文件
srm res -f f_a3b4c5 d_789abc
# 自定义路径恢复：将项恢复到指定目录
srm res -t /home/user/restore_dir f_a3b4c5
# 查看帮助
srm res --help
```

#### 执行结果
```
✅ Restored: f_a3b4c5 → /home/user/test.txt
```

### list（列出回收站内容）
#### 用法
查看回收站中所有项的状态，包括短ID、原路径、大小、过期时间、是否过期，支持详细模式和仅显示过期项。
```bash
srm list [OPTIONS]
# 别名：srm ls（推荐）
```

#### 参数
| 参数        | 简写 | 类型 | 说明                                                | 默认值 |
| ----------- | ---- | ---- | --------------------------------------------------- | ------ |
| `--expired` | -    | 布尔 | 仅显示已过期的回收站项                              | 禁用   |
| `--verbose` | `-v` | 布尔 | 详细模式：显示全量元数据（权限/UID/GID/删除时间等） | 禁用   |
| `--help`    | `-h` | -    | 查看该命令详细帮助                                  | -      |

#### 示例
```bash
# 基础列出：简易视图，显示所有项（活跃+过期）
srm ls
# 仅列出：已过期的回收站项
srm ls --expired
# 详细列出：显示所有项的全量元数据
srm ls -v
# 查看帮助
srm ls --help
```

#### 执行结果（简易视图）
```
📦 Active items (2):
🆔 SHORT      ORIGINAL PATH                             EXPIRES IN    SIZE
------------ ----------------------------------------- ------------ ---------------
f_a3b4c5     /home/user/test.txt                       6d 12h        1.2 MB
d_789abc     /home/user/temp_dir                       14d 5h        890 MB (dir)

🗑️  Expired items (1):
🆔 SHORT      ORIGINAL PATH                             EXPIRED       SIZE
------------ ----------------------------------------- ------------ ---------------
l_xyz123     /home/user/link_to_data                   2d 3h ago     0 B (symlink)
```

### clean（清理回收站）
#### 用法
清理回收站中**已过期的项**（默认）或**所有项**，永久删除并释放磁盘空间，支持批量清理。
```bash
srm clean [OPTIONS]
# 别名：srm cln（推荐）
```

#### 参数
| 参数     | 简写 | 类型 | 说明                                 | 默认值 |
| -------- | ---- | ---- | ------------------------------------ | ------ |
| `--all`  | `-a` | 布尔 | 清理所有项（无论是否过期），谨慎使用 | 禁用   |
| `--help` | `-h` | -    | 查看该命令详细帮助                   | -      |

#### 示例
```bash
# 基础清理：仅删除已过期的项（推荐）
srm cln
# 强制清理：删除回收站所有项（不可恢复）
srm cln -a
# 查看帮助
srm cln --help
```

#### 执行结果
```
🗑️  Cleaned: l_xyz123 (/home/user/link_to_data)
✅ Clean completed! 1 item(s) removed (0 B total)
```

### empty（永久清空回收站）
#### 用法
永久删除回收站中**所有项**及对应元数据，**操作不可恢复**，默认需要手动确认，支持跳过确认。
```bash
srm empty [OPTIONS]
# 别名：srm empty（无简写，防止误操作）
```

#### 参数
| 参数     | 简写 | 类型 | 说明                         | 默认值 |
| -------- | ---- | ---- | ---------------------------- | ------ |
| `--yes`  | `-y` | 布尔 | 跳过确认提示，直接清空回收站 | 禁用   |
| `--help` | `-h` | -    | 查看该命令详细帮助           | -      |

#### 示例
```bash
# 基础清空：需要手动确认（推荐，防止误操作）
srm empty
# 强制清空：跳过确认，直接删除所有项（谨慎使用）
srm empty -y
# 查看帮助
srm empty --help
```

#### 执行结果
```
⚠️  Empty trash permanently? This cannot be undone! [y/N]: y
✅ Trash emptied! 3 item(s) permanently deleted (891.2 MB total)
```

### 全局帮助
查看所有命令和全局选项说明：
```bash
# 查看全局帮助
srm --help
# 查看具体子命令帮助
srm [COMMAND] --help
```

## 自动清理与Systemd服务配置
针对**定期清理过期回收站数据**的需求，将`srm clean`配置为Systemd服务+定时器，实现开机自启、定时自动执行，步骤如下：

### 步骤1：创建Systemd服务文件
新建 `/etc/systemd/system/srm.service`，内容如下：
```ini
[Unit]
Description=Safe RM Trash Auto Clean Service
After=network.target local-fs.target
Documentation=man:srm(1)

[Service]
Type=oneshot  # 单次执行，配合timer触发
ExecStart=/usr/local/bin/srm clean  # 核心命令：清理过期项
User=root  # 普通用户使用请改为对应用户名（如user）
Group=root  # 对应用户组
WorkingDirectory=/tmp
Restart=no  # 无需重启
PrivateTmp=true  # 私有临时目录，提高安全性

[Install]
WantedBy=multi-user.target
```

### 步骤2：创建Systemd定时器文件
新建 `/etc/systemd/system/srm.timer`（控制执行周期），内容如下：
```ini
[Unit]
Description=Timer for Safe RM Trash Auto Clean
Requires=srm.service

[Timer]
OnCalendar=daily  # 执行周期：每天执行（可自定义）
Persistent=true   # 系统关机错过执行，开机后自动补执行
AccuracySec=1min  # 执行精度：1分钟内
Unit=srm.service  # 关联的服务文件

[Install]
WantedBy=timers.target
```

### 步骤3：重载配置并启用服务
```bash
# 重载Systemd配置
sudo systemctl daemon-reload
# 启用并启动定时器（核心：开机自启）
sudo systemctl enable --now srm.timer
# 验证定时器状态
sudo systemctl list-timers srm.timer
```

### 核心操作命令
```bash
# 手动执行一次清理
sudo systemctl start srm.service
# 查看服务执行日志
journalctl -u srm.service -f
# 查看定时器状态
sudo systemctl status srm.timer
# 停止并禁用定时器
sudo systemctl disable --now srm.timer
```

### 执行周期自定义
修改 `OnCalendar` 参数可自定义清理频率，常见配置：
- `hourly`：每小时执行
- `daily`：每天执行（默认）
- `weekly`：每周执行
- `monthly`：每月执行
- 自定义时间：`*-*-* 02:00:00`（每天凌晨2点执行）

## 安全替代原生rm（按用户独立生效）
实现**单个用户**使用`srm`替代原生`rm`，**不影响其他用户和系统全局`rm`**，核心通过Shell别名实现，支持`bash`/`zsh`，完全保留原生`rm`使用习惯。

### 方式1：临时生效（当前终端）
```bash
# bash/zsh 通用，rm映射为srm del，rmf映射为强制删除
alias rm='srm del'
alias rmf='srm del -f'
```
- 执行后，`rm test.txt` 等价于 `srm del test.txt`，关闭终端后别名失效。

### 方式2：永久生效（仅当前用户，推荐）
#### Bash用户
```bash
# 写入bash配置文件
echo 'alias rm="srm del"' >> $HOME/.bashrc
echo 'alias rmf="srm del -f"' >> $HOME/.bashrc
# 加载配置生效
source $HOME/.bashrc
```

#### Zsh用户
```bash
# 写入zsh配置文件
echo 'alias rm="srm del"' >> $HOME/.zshrc
echo 'alias rmf="srm del -f"' >> $HOME/.zshrc
# 加载配置生效
source $HOME/.zshrc
```

### 关键说明
1. **用户隔离**：仅当前用户的Shell生效，其他用户（包括root）仍使用原生`rm`；
2. **系统安全**：系统级脚本/命令仍调用原生`rm`，不会因`srm`故障影响系统运行；
3. **还原原生rm**：删除配置文件中的别名行即可，无任何残留：
   ```bash
   # Bash用户
   sed -i '/alias rm=/d' $HOME/.bashrc && source $HOME/.bashrc
   # Zsh用户
   sed -i '/alias rm=/d' $HOME/.zshrc && source $HOME/.zshrc
   ```

## 配置说明
### 核心配置设计
`srm`采用**硬编码默认配置+无外置配置文件**的设计（源码中通过常量定义），无需手动修改配置，开箱即用，所有默认值均为行业最佳实践，核心常量如下：

| 常量名                     | 取值        | 核心说明                           |
| -------------------------- | ----------- | ---------------------------------- |
| `DEFAULT_EXPIRE_DAYS`      | 7           | 默认过期天数                       |
| `MAX_LOG_AGE_DAYS`         | 30          | 日志自动轮转保留天数               |
| `PROTECTED_PATHS`          | 8个系统路径 | `/bin`/`/etc`/`/usr`等核心保护路径 |
| `SHORT_ID_LENGTH`          | 6           | 短ID字符长度（带类型前缀）         |
| `PROGRESS_THRESHOLD_BYTES` | 100MB       | 显示进度条的文件大小阈值           |
| `MAX_FILE_SPACE_RATIO`     | 0.8         | 单文件最大占用可用空间比例（80%）  |
| `MMAP_CHUNK_SIZE`          | 4MB         | 大文件mmap分块传输大小             |
| `MAX_RECURSION_DEPTH`      | 1000        | 目录遍历最大深度（防止栈溢出）     |

### 自定义配置（源码修改）
若需调整默认配置，修改`main.rs`中的常量后重新编译即可：
1. 打开`main.rs`，找到文件顶部的常量定义区域；
2. 修改对应常量值（如将`DEFAULT_EXPIRE_DAYS`改为15）；
3. 重新编译+UPX压缩：`cargo build --release && upx --best --lzma target/release/srm`；
4. 覆盖原二进制：`sudo cp target/release/srm /usr/local/bin/`。

### 核心数据目录结构
所有数据均存储在**`srm`可执行文件同级的`.srm`目录**中，自动创建，权限严格隔离：
```
.srm/
├── trash/        # 回收站：存储被删除的文件/目录，权限0700
├── meta/         # 元数据：JSON格式存储删除项信息，原子化写入，权限0700
└── srm.log       # 审计日志：JSON格式，自动轮转，权限0600
```
- 迁移回收站数据：直接复制整个`.srm`目录到新的`srm`可执行文件同级即可，元数据和日志自动保留。

## 性能优化特性
源码针对**大文件/大目录/跨文件系统操作**做了多层极致优化，适配TB级文件操作，核心优化点如下：
1. **同文件系统0拷贝**：源文件与回收站在同一文件系统时，直接执行`rename`系统调用，瞬间完成，无数据拷贝；
2. **跨文件系统CoW写时复制**：Linux下自动检测Btrfs/XFS/ZFS等支持CoW的文件系统，通过`ioctl FICLONE`实现无数据拷贝，比普通拷贝快10倍以上；
3. **硬链接优先策略**：同文件系统下若CoW不支持，自动尝试创建硬链接，避免数据拷贝；
4. **大文件mmap分块传输**：对>10MB的文件，使用`mmap2`将文件映射到内存，按4MB分块传输，减少系统调用，提高吞吐量；
5. **迭代式目录遍历**：采用栈实现目录迭代遍历，避免递归栈溢出，支持最大1000级目录深度；
6. **实时进度追踪**：大文件（>100MB）/大目录（>5项）操作时，显示实时进度条，包含耗时、吞吐量、剩余时间；
7. **批量磁盘空间校验**：删除前批量校验磁盘空间，避免多次IO操作，提高批量删除效率；
8. **Rust编译极致优化**：`Cargo.toml`中开启`opt-level=3`、`lto=fat`、`strip=true`，编译出的二进制体积小、执行效率高；
9. **UPX压缩优化**：编译产物支持UPX极致压缩，体积减小60%+，不损失执行性能，便于分发部署。

## 日志与审计
### 日志核心特点
1. **JSON标准格式**：所有日志均为标准JSON格式，便于自动化解析、审计和日志收集工具（如ELK）对接；
2. **自动轮转**：日志保留30天，自动清理30天前的日志，避免日志文件过大；
3. **严格权限隔离**：日志文件权限为`0600`，仅当前用户可读取，防止审计数据泄露；
4. **全量操作记录**：记录所有操作类型，包括删除、恢复、清理、空回收站、操作中断、跳过/失败等；
5. **元数据完整**：每条日志包含**毫秒级时间戳、日志级别、操作信息、详细元数据**（路径、短ID、大小、权限、UID/GID、执行结果等）。

### 日志存储路径
```bash
# 日志文件位于.srm目录中，完整路径可通过以下命令获取
$(dirname $(which srm))/.srm/srm.log
# 示例：若srm在/usr/local/bin/，则日志路径为/usr/local/bin/.srm/srm.log
```

### 日志内容示例
```json
{
  "timestamp": "2026-01-30 15:20:30.123",
  "level": "INFO",
  "message": "File deleted",
  "details": {
    "action": "delete",
    "short_id": "f_a3b4c5",
    "trash_id": "test.txt_1738238430123456789",
    "original_path": "/home/user/test.txt",
    "backup_path": "/usr/local/bin/.srm/trash/test.txt_1738238430123456789",
    "file_type": "file",
    "size_bytes": 1258291,
    "permissions": "644",
    "expire_days": 7,
    "forced": false,
    "duration_ms": 120
  }
}
```

### 日志查看与解析
```bash
# 实时查看日志
tail -f $(dirname $(which srm))/.srm/srm.log
# 格式化查看JSON日志（需安装jq）
jq . $(dirname $(which srm))/.srm/srm.log
# 筛选删除操作日志
jq 'select(.details.action == "delete")' $(dirname $(which srm))/.srm/srm.log
# 筛选错误日志
jq 'select(.level == "ERROR" or .level == "WARN")' $(dirname $(which srm))/.srm/srm.log
```

## 安全防护机制
`srm`内置多层安全防护机制，从根本上避免误删和系统损坏，核心防护点如下：
1. **系统路径强制保护**：默认禁止删除`/bin`、`/sbin`、`/etc`、`/usr`、`/lib`、`/lib64`、`/root`、`/boot`8个核心系统路径，需`-f`强制覆盖；
2. **路径遍历攻击防护**：默认禁止删除含`..`的路径（如`../etc/passwd`），防止恶意路径遍历，需`-f`强制覆盖；
3. **磁盘空间严格校验**：删除前检查目标文件系统可用空间，单文件最大占用80%可用空间，批量删除校验总空间，防止磁盘占满；
4. **软链目标安全校验**：检查软链指向的目标路径，若指向系统保护路径，默认禁止删除，需`-f`强制覆盖；
5. **中断安全自动回滚**：Ctrl+C中断操作时，正在执行的删除任务会自动回滚，将已复制的文件恢复到原路径，避免文件丢失；
6. **原子化元数据写入**：元数据采用“先写临时文件，再重命名”的原子化操作，防止进程崩溃导致元数据损坏；
7. **严格的权限控制**：回收站、日志、元数据目录/文件分别设置`0700/0600`权限，仅当前用户可访问，避免越权查看/修改/恢复；
8. **不存在文件自动跳过**：删除时自动跳过不存在的文件，不抛出错误，提高批量操作稳定性。

## 常见问题
### Q1：删除的文件存储在哪里？如何迁移回收站数据？
A：存储在`srm`可执行文件同级的`.srm/trash`目录中；迁移时直接复制整个`.srm`目录到新的`srm`可执行文件同级即可，元数据和日志会自动保留。

### Q2：忘记短ID了，如何恢复文件？
A：执行`srm ls`查看回收站所有项的短ID和原路径，找到对应项后用`srm res 短ID`恢复即可。

### Q3：srm是否支持跨文件系统删除？
A：支持，跨文件系统时会自动采用**CoW写时复制**（支持的文件系统）或**mmap分块复制**，并显示实时进度条，性能优于原生`mv`。

### Q4：为什么执行`srm del`后，原文件路径的磁盘空间没有释放？
A：因为`srm`是将文件移动到回收站，并非永久删除，磁盘空间会在执行`srm cln`（清理过期）或`srm empty`（清空）后释放。

### Q5：UPX压缩后的二进制是否会影响执行性能？
A：不会，UPX是无损压缩，运行时会自动将二进制解压缩到内存，仅首次启动耗时微增（毫秒级），后续执行与未压缩版本一致。

### Q6：srm是否支持大文件（如100GB）删除？
A：支持，针对大文件做了**mmap分块传输**和**实时进度追踪**，支持中断回滚，不会因内存不足导致崩溃。

### Q7：普通用户能否删除root用户的文件？
A：不能，受Linux文件系统权限控制，普通用户仅能删除自己拥有读写权限的文件，与原生`rm`一致。

### Q8：Systemd定时器不执行怎么办？
A：1. 检查定时器状态：`sudo systemctl status srm.timer`；2. 查看执行日志：`journalctl -u srm.service -f`；3. 确认`srm`路径正确（`/usr/local/bin/srm`）；4. 重新重载配置：`sudo systemctl daemon-reload && sudo systemctl restart srm.timer`。

### Q9：如何查看srm的所有操作日志？
A：日志路径为`$(dirname $(which srm))/.srm/srm.log`，可通过`tail -f`实时查看，或用`jq`工具格式化解析JSON日志。

### Q10：srm的回收站是否有大小限制？
A：无硬性大小限制，删除前会校验目标文件系统可用空间，单文件最大占用80%可用空间，批量删除校验总空间，防止磁盘占满。

---
**版本**：v1.2.1  
**作者**：Meitao Lin <mtl>  
**许可证**：MIT  
**开发语言**：Rust 2021 Edition  
**构建工具**：Cargo 1.93.0 (083ac5135 2025-12-15)  
**核心优化**：UPX极致压缩（--best --lzma）

# SRM 终端自动补全（Bash/Zsh）
基于 `safe-srm` 源码的命令结构，实现 Bash/Zsh 终端下 `srm` 命令自动补全，支持子命令、参数、动态路径/ID 提示。

## Bash 补全脚本（srm-completion.bash）
```bash
#!/bin/bash
_srm_completions() {
    local cur prev words cword
    _init_completion || return

    # 核心子命令
    local commands="delete restore list clean help version"
    # 全局/子命令专属选项
    local global_opts="-h --help -V --version -f --force -e --expire-days -v --verbose"
    local delete_opts="-f --force -e --expire-days"
    local restore_opts="-i --id -p --path"
    local clean_opts="-a --all -d --days -n --dry-run"
    local list_opts="-a --all -l --long -s --short"

    # 补全逻辑
    case $prev in
        srm) COMPREPLY=($(compgen -W "$commands $global_opts" -- "$cur")) ;;
        delete) COMPREPLY=($(compgen -W "$delete_opts $(ls -1 2>/dev/null)" -- "$cur")) ;;
        restore)
            # 提取trash中可恢复的short_id
            local meta_dir=$(dirname $(which srm))/.srm/meta
            local restore_ids=$(ls -1 "$meta_dir"/*.meta 2>/dev/null | sed 's/\.meta$//' | xargs -I {} basename {})
            COMPREPLY=($(compgen -W "$restore_opts $restore_ids" -- "$cur")) ;;
        clean) COMPREPLY=($(compgen -W "$clean_opts" -- "$cur")) ;;
        list) COMPREPLY=($(compgen -W "$list_opts" -- "$cur")) ;;
        -e|--expire-days|-d|--days) COMPREPLY=($(compgen -W "1 3 7 14 30" -- "$cur")) ;;
        *) COMPREPLY=($(compgen -o filenames -W "$global_opts" -- "$cur")) ;;
    esac
    return 0
}
complete -F _srm_completions srm
```

### 生效方式
```bash
# 临时生效
source /path/to/srm-completion.bash
# 永久生效
echo "source /path/to/srm-completion.bash" >> ~/.bashrc && source ~/.bashrc
```

## Zsh 补全脚本（srm-completion.zsh）
```zsh
#compdef srm
local curcontext="$curcontext" state line
typeset -A opt_args

# 子命令定义
local commands=(
    'delete:Move files/dirs to safe trash'
    'restore:Restore files from trash'
    'list:List trashed items'
    'clean:Clean expired items in trash'
    'help:Show help'
    'version:Show version'
)

# 选项定义
local global_opts=(
    '(-h --help)'{-h,--help}'[Show help]'
    '(-V --version)'{-V,--version}'[Show version]'
    '(-f --force)'{-f,--force}'[Override safety checks]'
    '(-e --expire-days)'{-e,--expire-days}'[Set expire days]:days:(1 3 7 14 30)'
)
local delete_opts=('(-f --force)'{-f,--force}'[Override safety checks]' '(-e --expire-days)'{-e,--expire-days}'[Set expire days]:days:(1 3 7 14 30)')
local restore_opts=(
    '(-i --id)'{-i,--id}'[Restore by short ID]:short_id:($(_srm_get_trashed_ids))'
    '(-p --path)'{-p,--path}'[Restore original path]:path:($(_srm_get_trashed_paths))'
)
local clean_opts=(
    '(-a --all)'{-a,--all}'[Clean all items]'
    '(-d --days)'{-d,--days}'[Clean items older than N days]:days:(1 3 7 14 30)'
    '(-n --dry-run)'{-n,--dry-run}'[Dry run (no deletion)]'
)

# 动态获取trash中的ID/路径
_srm_get_trashed_ids() {
    local meta_dir=$(dirname $(which srm))/.srm/meta
    ls -1 "$meta_dir"/*.meta 2>/dev/null | sed 's/\.meta$//' | xargs -I {} basename {}
}
_srm_get_trashed_paths() {
    local meta_dir=$(dirname $(which srm))/.srm/meta
    for meta in "$meta_dir"/*.meta; do [ -f "$meta" ] && jq -r '.original_path' "$meta" 2>/dev/null; done | sort -u
}

# 补全逻辑
_arguments -C \
    '1: :->command' \
    '*: :->args' && return 0

case $state in
    command) _describe -t commands 'srm commands' commands ;;
    args)
        local cmd=${words[2]}
        case $cmd in
            delete) _arguments $delete_opts '*:file:->_path_files' ;;
            restore) _arguments $restore_opts ;;
            clean) _arguments $clean_opts ;;
            list) _arguments '(-a --all)'{-a,--all}'[Show all items]' '(-l --long)'{-l,--long}'[Long format]' '(-s --short)'{-s,--short}'[Short IDs only]' ;;
            *) _arguments $global_opts '*:file:->_path_files' ;;
        esac ;;
esac
```

### 生效方式
```zsh
# 临时生效
source /path/to/srm-completion.zsh
# 永久生效
echo "source /path/to/srm-completion.zsh" >> ~/.zshrc && source ~/.zshrc
```

### 核心特性
1. 子命令补全：`srm ` 按 Tab 提示 `delete/restore/list/clean/help/version`；
2. 选项补全：`srm delete -` 按 Tab 提示 `-f/--force -e/--expire-days`；
3. 动态值补全：`srm restore -i ` 自动提示 trash 中的 short_id；
4. 路径补全：`srm delete ` 自动补全当前目录文件/目录；
5. 常用值提示：过期天数自动提示 `1/3/7/14/30`。

# Zsh 原生补全插件：_srm 脚本
Zsh 补全插件需遵循其原生规范，文件命名为 `_srm`（无后缀），放置到 Zsh 补全目录后可被自动加载，以下是完整实现：

## _srm 补全脚本（核心文件）
```zsh
#compdef srm
# ------------------------------------------------------------------------------
# Description: Zsh 原生补全脚本 for safe-srm (srm)
# Author: Custom
# Version: 1.0
# ------------------------------------------------------------------------------

# 初始化上下文
local curcontext="$curcontext" state line
typeset -A opt_args

# -------------------------- 辅助函数：动态获取Trash数据 --------------------------
# 获取回收站中文件的short ID
_srm_get_trashed_ids() {
    local meta_dir="${0:A:h:h}/.srm/meta"  # 适配srm安装路径（可根据实际调整）
    [[ -d "$meta_dir" ]] || return 1
    ls -1 "$meta_dir"/*.meta 2>/dev/null | sed -E 's/\.meta$//' | xargs -I {} basename {}
}

# 获取回收站中文件的原始路径
_srm_get_trashed_paths() {
    local meta_dir="${0:A:h:h}/.srm/meta"
    [[ -d "$meta_dir" && -x "$(command -v jq)" ]] || return 1
    for meta in "$meta_dir"/*.meta; do
        [[ -f "$meta" ]] && jq -r '.original_path' "$meta" 2>/dev/null
    done | sort -u
}

# -------------------------- 补全规则定义 --------------------------
# 1. 子命令列表（key: 命令名，value: 描述）
local -a commands
commands=(
    'delete:将文件/目录移入安全回收站'
    'restore:从回收站恢复文件/目录'
    'list:列出回收站中的所有项'
    'clean:清理回收站中过期的项'
    'help:显示帮助信息'
    'version:显示版本信息'
)

# 2. 全局选项（所有子命令通用）
local -a global_opts
global_opts=(
    '(-h --help)'{-h,--help}'[显示帮助信息]'
    '(-V --version)'{-V,--version}'[显示版本信息]'
    '(-v --verbose)'{-v,--verbose}'[详细输出模式]'
)

# 3. 子命令专属选项
local -a delete_opts restore_opts clean_opts list_opts
delete_opts=(
    '(-f --force)'{-f,--force}'[跳过安全检查强制删除]'
    '(-e --expire-days)'{-e,--expire-days}'[设置文件过期天数]:过期天数:(1 3 7 14 30 90)'
)
restore_opts=(
    '(-i --id)'{-i,--id}'[通过short ID恢复文件]:Short ID:($(_srm_get_trashed_ids))'
    '(-p --path)'{-p,--path}'[通过原始路径恢复文件]:原始路径:($(_srm_get_trashed_paths))'
)
clean_opts=(
    '(-a --all)'{-a,--all}'[清理回收站所有项（忽略过期时间）]'
    '(-d --days)'{-d,--days}'[清理N天前的过期项]:天数:(1 3 7 14 30 90)'
    '(-n --dry-run)'{-n,--dry-run}'[模拟清理（不实际删除）]'
)
list_opts=(
    '(-a --all)'{-a,--all}'[显示所有回收站项（含隐藏）]'
    '(-l --long)'{-l,--long}'[长格式输出（显示详细信息）]'
    '(-s --short)'{-s,--short}'[仅显示short ID]'
)

# -------------------------- 核心补全逻辑 --------------------------
_arguments -C \
    ':子命令:->command' \
    '*::参数:->args' && return 0

# 第一步：补全子命令（srm 后第一个参数）
if [[ $state == command ]]; then
    _describe -t commands 'srm 子命令' commands
    return 0
fi

# 第二步：根据子命令补全后续参数/选项
local cmd="${words[2]}"  # 获取已输入的子命令
case $cmd in
    delete)
        _arguments \
            $global_opts \
            $delete_opts \
            '*:文件/目录:->_path_files'  # 补全本地文件路径
        ;;
    restore)
        _arguments \
            $global_opts \
            $restore_opts
        ;;
    clean)
        _arguments \
            $global_opts \
            $clean_opts
        ;;
    list)
        _arguments \
            $global_opts \
            $list_opts
        ;;
    help|version)
        _arguments $global_opts
        ;;
    *)
        _arguments $global_opts \
            '*:文件/目录:->_path_files'
        ;;
esac

return 0
```

## 插件生效步骤
### 步骤 1：放置脚本到 Zsh 补全目录
Zsh 补全目录优先级：
1. 自定义目录（推荐）：`~/.zsh/completions/`
2. 系统目录：`/usr/share/zsh/site-functions/`

```zsh
# 1. 创建自定义补全目录（若不存在）
mkdir -p ~/.zsh/completions

# 2. 将 _srm 脚本放入该目录
cp /path/to/your/_srm ~/.zsh/completions/

# 3. 设置权限
chmod +x ~/.zsh/completions/_srm
```

### 步骤 2：配置 Zsh 加载补全目录
编辑 `~/.zshrc`，添加以下内容：
```zsh
# 启用补全功能
autoload -Uz compinit && compinit

# 添加自定义补全目录到Zsh搜索路径
fpath=(~/.zsh/completions $fpath)
```

### 步骤 3：生效配置
```zsh
# 重新加载zsh配置
source ~/.zshrc

# 强制重建补全缓存（可选，首次配置建议执行）
compinit -u
```

## 核心特性
1. **原生兼容**：遵循 Zsh `compdef` 规范，支持 `compinit` 自动加载；
2. **动态补全**：`srm restore -i ` 自动提示回收站中的 short ID，`-p` 提示原始路径；
3. **路径补全**：`srm delete ` 自动补全本地文件/目录；
4. **选项提示**：所有参数/选项带中文描述，Tab 补全时直观显示；
5. **版本适配**：兼容 Zsh 5.0+ 主流版本。

## 测试验证
```zsh
# 测试子命令补全
srm [Tab]  # 提示 delete restore list clean help version

# 测试 delete 选项补全
srm delete -[Tab]  # 提示 -f --force -e --expire-days

# 测试 restore 动态ID补全
srm restore -i [Tab]  # 提示回收站中的short ID
```
