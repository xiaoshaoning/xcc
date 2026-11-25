"""Read a PE file's export table: every name it exports, one per line, sorted.

    python3 tools/pe_exports.py C:/Windows/System32/ucrtbase.dll

It exists to *make* `src/back/imports.rs`, which is the list of names the C runtime exports - a
capture of a fact the DLL owns, so that the linker can tell a name the runtime has from a typo
without reading a DLL at link time. Regenerate with:

    python3 tools/pe_exports.py C:/Windows/System32/ucrtbase.dll > names.txt
"""
import struct
import sys

# The export directory's own layout, relative to its start: the count of names and where the name
# pointers are. Both are all a caller needs; the ordinals and the addresses belong to the loader.
COUNT_AT = 24
NAMES_AT = 32


def exports(path):
    with open(path, 'rb') as handle:
        data = handle.read()
    pe = struct.unpack_from('<I', data, 0x3C)[0]
    if data[pe:pe + 4] != b'PE\0\0':
        raise SystemExit('%s is not a PE file' % path)
    sections = struct.unpack_from('<H', data, pe + 6)[0]
    optional = pe + 24
    magic = struct.unpack_from('<H', data, optional)[0]
    directories = optional + (112 if magic == 0x20B else 96)
    export_rva, _ = struct.unpack_from('<II', data, directories)
    if export_rva == 0:
        return []

    # Each section: where it is in memory, how far it reaches, and where its bytes are in the file.
    table = optional + struct.unpack_from('<H', data, pe + 20)[0]
    mapped = []
    for index in range(sections):
        entry = table + index * 40
        virtual_size, virtual_address = struct.unpack_from('<II', data, entry + 8)
        raw_size, raw_offset = struct.unpack_from('<II', data, entry + 16)
        mapped.append((virtual_address, max(virtual_size, raw_size), raw_offset))

    def offset_of(rva):
        for virtual_address, extent, raw_offset in mapped:
            if virtual_address <= rva < virtual_address + extent:
                return raw_offset + (rva - virtual_address)
        raise SystemExit('rva %#x is in no section' % rva)

    directory = offset_of(export_rva)
    count = struct.unpack_from('<I', data, directory + COUNT_AT)[0]
    names_at = offset_of(struct.unpack_from('<I', data, directory + NAMES_AT)[0])
    found = []
    for index in range(count):
        name_rva = struct.unpack_from('<I', data, names_at + index * 4)[0]
        start = offset_of(name_rva)
        found.append(data[start:data.index(b'\0', start)])
    return found


def main(path):
    names = sorted({name for name in exports(path)})
    # ASCII only: a name that is not is written with its escapes rather than crashing a console.
    text = '\n'.join(name.decode('ascii', 'backslashreplace') for name in names)
    sys.stdout.buffer.write((text + '\n').encode('ascii'))


if __name__ == '__main__':
    main(sys.argv[1])
