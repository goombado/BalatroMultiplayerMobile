"""Checks that built OpenSSL libraries export every function MultiplayerAPI's openssl_ffi.lua declares.

usage: python3 -I check-symbols.py <openssl_ffi.lua> <nm-tool> <lib> [<lib> ...]

Only the cdef blocks that bind OpenSSL are checked (the libc helpers such as fcntl/ioctlsocket are
skipped): a block counts if it declares an SSL_/EVP_/ERR_/RAND_/OPENSSL_/HMAC/TLS_ function.
"""
import re
import subprocess
import sys

OPENSSL_PREFIX = re.compile(r'^(SSL_|SSL$|EVP_|ERR_|RAND_|OPENSSL_|HMAC|TLS_|CRYPTO_|BIO_|X509|PEM_|OBJ_|DTLS_)')


def declared_functions(lua_src):
    names = set()
    for block in re.findall(r'cdef\(\s*\[(=*)\[(.*?)\]\1\]', lua_src, re.S):
        body = re.sub(r'//[^\n]*|/\*.*?\*/', '', block[1], flags=re.S)
        for m in re.finditer(r'([A-Za-z_][A-Za-z0-9_]*)\s*\(([^;{}]*)\)\s*;', body):
            name = m.group(1)
            if name.startswith('typedef') or '(*' in m.group(0):
                continue
            if OPENSSL_PREFIX.match(name):
                names.add(name)
    return names


def exported(nm, lib):
    with open(lib, 'rb') as f:
        elf = f.read(4) == b'\x7fELF'
    # ELF: read the dynamic table (release .so files are stripped) and drop version suffixes (SSL_new@@OPENSSL_3.0.0).
    out = subprocess.run([nm, '-D' if elf else '-g', '--defined-only', lib], capture_output=True, text=True, check=True).stdout
    syms = set()
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 3:
            name = parts[2].split('@')[0]
            syms.add(name if elf or not name.startswith('_') else name[1:])
    return syms


def main():
    lua_path, nm, libs = sys.argv[1], sys.argv[2], sys.argv[3:]
    with open(lua_path, encoding='utf-8') as f:
        wanted = declared_functions(f.read())
    have = set()
    for lib in libs:
        have |= exported(nm, lib)
    missing = sorted(wanted - have)
    print(f'{len(wanted)} OpenSSL functions declared, {len(wanted) - len(missing)} exported')
    if missing:
        print('MISSING: ' + ', '.join(missing))
        sys.exit(1)


if __name__ == '__main__':
    main()
