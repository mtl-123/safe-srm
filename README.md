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
