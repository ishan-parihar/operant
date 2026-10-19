#!/usr/bin/env bash
# One libclang resolution for every build script (dev-env.sh, check.sh,
# provision-build-deps.sh).
#
# WHY THIS FILE EXISTS. bindgen dlopens libclang and globs a directory for
# 'libclang.so', 'libclang-*.so', 'libclang.so.*', 'libclang-*.so.*'. Three
# things then go wrong, and each one cost a build:
#
#   1. A distro that ships only 'libclang.so.N.N' satisfies none of those
#      globs, so a directory that holds a perfectly good libclang can still
#      fail to load one.
#   2. A DANGLING symlink (a link to a libclang version the box later
#      removed) passes a naive "does this path exist" test under some shells
#      and fails to load under bindgen. "The directory exists" is not the
#      test; "a globbable name in it resolves to a real file" is.
#   3. A box that ships libclang only under '/usr/lib/llvm-N/lib' is missed by
#      a flat '/usr/lib' search, so the provisioner downloads a Debian .deb
#      it does not need.
#
# The three scripts used to resolve libclang three different ways, and none
# of them was correct: dev-env.sh fell through to a directory that did not
# exist and only printed a warning, so the build died two minutes later in
# bindgen with a message that named neither the cause nor the fix.
#
# Usage:
#   source scripts/libclang-resolve.sh
#   LIBCLANG_PATH="$(resolve_libclang "$LOCAL_DIR")"   # empty if none found
#
# Sourced, never executed: it defines functions and sets no variables of its
# own, so sourcing it cannot change a caller's shell state.

# _libclang_dir_has_lib DIR
# True when DIR holds a file bindgen can actually dlopen under one of the
# names it globs. A dangling symlink is rejected: `-e` follows symlinks, so
# a link to a removed target is not a file that exists.
_libclang_dir_has_lib() {
    local dir="${1:-}"
    [ -n "$dir" ] && [ -d "$dir" ] || return 1
    local f
    for f in "$dir"/libclang.so "$dir"/libclang-*.so "$dir"/libclang.so.* "$dir"/libclang-*.so.*; do
        # shellcheck disable=SC2144 # the loop is over globs, not a literal list
        if [ -e "$f" ]; then
            return 0
        fi
    done
    return 1
}

# resolve_libclang LOCAL_DIR
# Prints the first directory that holds a loadable libclang, or nothing.
#
# Order:
#   1. an explicit LIBCLANG_PATH, when it actually holds one (a caller that
#      set it to a bad path is not obeyed into a broken build)
#   2. the provisioned link directory, LOCAL_DIR/libclang
#   3. the system directories, including the '/usr/lib/llvm-N/lib' layout
#   4. the extracted Debian .deb directory, the last-resort fallback
resolve_libclang() {
    local local_dir="${1:-}"
    local cand

    if [ -n "${LIBCLANG_PATH:-}" ] && _libclang_dir_has_lib "$LIBCLANG_PATH"; then
        printf '%s\n' "$LIBCLANG_PATH"
        return 0
    fi

    if [ -n "$local_dir" ] && _libclang_dir_has_lib "$local_dir/libclang"; then
        printf '%s\n' "$local_dir/libclang"
        return 0
    fi

    # The system directories. The llvm-N layout is the one most distributions
    # use and the one a flat '/usr/lib' search misses.
    for cand in /usr/lib /usr/lib/llvm*/lib /usr/lib64 /usr/local/lib; do
        if _libclang_dir_has_lib "$cand"; then
            printf '%s\n' "$cand"
            return 0
        fi
    done

    if [ -n "$local_dir" ] && _libclang_dir_has_lib "$local_dir/libclang_extract/usr/lib/x86_64-linux-gnu"; then
        printf '%s\n' "$local_dir/libclang_extract/usr/lib/x86_64-linux-gnu"
        return 0
    fi

    return 1
}

# libclang_link_target DIR
# Prints the system libclang that DIR/libclang.so should point at, or nothing.
# Used by the provisioner to repair a link whose target has gone away.
libclang_link_target() {
    local local_dir="${1:-}"
    local cand
    # The plain name first, then any versioned variant, then the llvm-N
    # layout. `/usr/lib/*/libclang.so.*.*` is one level too shallow for
    # '/usr/lib/llvm-N/lib/', which is why that path is spelled out.
    for cand in /usr/lib/libclang.so /usr/lib/libclang.so.*.* /usr/lib/*/libclang.so.*.* /usr/lib/llvm*/lib/libclang.so.*.*; do
        if [ -e "$cand" ]; then
            printf '%s\n' "$cand"
            return 0
        fi
    done
    return 1
}
