#!/usr/bin/env python3
"""Capture a private ARM Android consumerir HAL in an offline emulator.

Requires pyelftools and unicorn; no guest syscall reaches the host or device.
Original HAL binaries/captures belong outside Git. See --help for inputs.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

from elftools.elf.elffile import ELFFile
from unicorn import Uc, UC_ARCH_ARM, UC_MODE_ARM, UC_HOOK_CODE, UC_HOOK_INTR
from unicorn.arm_const import (
    UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_R2, UC_ARM_REG_R3,
    UC_ARM_REG_SP, UC_ARM_REG_LR, UC_ARM_REG_PC, UC_ARM_REG_C1_C0_2,
    UC_ARM_REG_FPEXC,
)

BASE, STUB, HEAP, STACK = 0x100000, 0x200000, 0x400000, 0x1000000
REGS = [UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_R2, UC_ARM_REG_R3]


class Capture:
    def __init__(self, filename, properties):
        self.uc = Uc(UC_ARCH_ARM, UC_MODE_ARM)
        self.uc.mem_map(BASE, 0x100000)
        self.uc.mem_map(STUB, 0x10000)
        self.uc.mem_map(HEAP, 0x800000)
        self.uc.mem_map(STACK, 0x100000)
        self.uc.reg_write(UC_ARM_REG_C1_C0_2, 0xF00000)
        self.uc.reg_write(UC_ARM_REG_FPEXC, 0x40000000)
        self.heap = HEAP
        self.properties = properties
        self.events = []
        self.writes = []
        self.stubs = {}
        self.uc.hook_add(UC_HOOK_CODE, self.hook)
        self.uc.hook_add(UC_HOOK_INTR, self.interrupt)
        self.stop = STUB + 0xF000
        with open(filename, "rb") as file:
            elf = ELFFile(file)
            if elf['e_machine'] != 'EM_ARM' or elf.elfclass != 32 or not elf.little_endian:
                raise ValueError("Expected a little-endian ARM32 ELF")
            for segment in elf.iter_segments():
                if segment['p_type'] == 'PT_LOAD':
                    self.uc.mem_write(BASE + segment['p_vaddr'], segment.data())
            syms = elf.get_section_by_name('.dynsym')
            self.hmi = BASE + syms.get_symbol_by_name('HMI')[0]['st_value']
            for section in elf.iter_sections():
                if section['sh_type'] != 'SHT_REL':
                    continue
                for rel in section.iter_relocations():
                    loc = BASE + rel['r_offset']
                    kind = rel['r_info_type']
                    sym = syms.get_symbol(rel['r_info_sym'])
                    if kind == 23:  # R_ARM_RELATIVE
                        value = BASE + self.word(loc)
                    elif kind in (21, 22, 2):  # GLOB_DAT, JUMP_SLOT, ABS32
                        if sym['st_shndx'] == 'SHN_UNDEF':
                            value = self.stub(sym.name)
                        else:
                            value = BASE + sym['st_value']
                        if kind == 2:
                            value += self.word(loc)
                    else:
                        raise ValueError(f"Unhandled relocation {kind}")
                    self.put(loc, value)

    def word(self, at):
        return struct.unpack('<I', self.uc.mem_read(at, 4))[0]

    def put(self, at, value):
        self.uc.mem_write(at, struct.pack('<I', value & 0xFFFFFFFF))

    def string(self, at):
        data = bytearray()
        for i in range(4096):
            byte = self.uc.mem_read(at+i, 1)[0]
            if not byte:
                return data.decode('utf-8', errors='replace')
            data.append(byte)
        raise ValueError("Unbounded string")

    def allocate(self, count):
        if count < 0 or count > 2**20 or self.heap + count + 16 >= HEAP + 0x800000:
            raise ValueError("Allocation limit exceeded")
        addr = self.heap
        self.heap += (count+15) & ~15
        return addr

    def stub(self, name):
        for addr, existing in self.stubs.items():
            if existing == name:
                return addr
        addr = STUB + 16 * len(self.stubs)
        self.stubs[addr] = name
        self.uc.mem_write(addr, bytes.fromhex('1eff2fe1'))  # ARM bx lr
        return addr

    def interrupt(self, uc, number, user):
        raise RuntimeError(f"Guest syscall/interrupt {number} denied")

    def hook(self, uc, addr, size, user):
        if addr == self.stop:
            uc.emu_stop()
            return
        if addr not in self.stubs:
            return
        name = self.stubs[addr]
        a, b, c, d = [uc.reg_read(r) for r in REGS]
        result = 0
        if name in ('malloc',):
            result = self.allocate(a)
        elif name.startswith('__aeabi_memclr'):
            uc.mem_write(a, bytes(b))
        elif name.startswith('__aeabi_memcpy'):
            uc.mem_write(a, bytes(uc.mem_read(b, c)))
        elif name == 'strcmp':
            result = (self.string(a) > self.string(b)) - (self.string(a) < self.string(b))
        elif name == 'atoi':
            result = int(self.string(a))
        elif name == 'ceilf':
            value = struct.unpack('<f', struct.pack('<I', a))[0]
            result = struct.unpack('<I', struct.pack('<f', math.ceil(value)))[0]
        elif name == 'property_get':
            key = self.string(a)
            value = self.properties.get(key, self.string(c) if c else '')
            uc.mem_write(b, value.encode()+b'\0')
            self.events.append({'property': key, 'value': value})
            result = len(value)
        elif name == '__open_2':
            path = self.string(a)
            if path != '/dev/irtx':
                raise RuntimeError(f"Unexpected guest open {path}")
            self.events.append({'open':path,'flags':b,'simulated_fd':42})
            result = 42
        elif name == 'ioctl':
            if a != 42:
                raise RuntimeError('Unexpected ioctl fd')
            # The sole stock query is GET_SOLUTTION_TYPE (_IOR R,1,int).
            # Simulate the measured HA100 solution1 response; deny unknown I/O.
            if b != 0x80045201:
                raise RuntimeError(f"Unsupported ioctl {b:#x}")
            self.put(c, 1)
            self.events.append({'ioctl':f'0x{b:08x}','arg':c,
                                'solution_type_reply':1})
        elif name == 'write':
            if a != 42 or c > 2**20:
                raise RuntimeError('Unexpected write')
            data = bytes(uc.mem_read(b,c))
            self.writes.append(data)
            self.events.append({'write_bytes':c,'sha256':hashlib.sha256(data).hexdigest()})
            result = c
        elif name == '__errno':
            result = HEAP + 0x7FF000
        elif name in ('__android_log_print','free','close','__cxa_finalize','__cxa_atexit','__register_atfork'):
            pass
        else:
            raise RuntimeError(f"Unimplemented guest import {name}")
        uc.reg_write(UC_ARM_REG_R0, result & 0xFFFFFFFF)
        uc.reg_write(UC_ARM_REG_PC, uc.reg_read(UC_ARM_REG_LR))

    def call(self, function, *args):
        self.uc.reg_write(UC_ARM_REG_SP, STACK+0xFF000)
        for reg, value in zip(REGS, args):
            self.uc.reg_write(reg, value)
        self.uc.reg_write(UC_ARM_REG_LR, self.stop)
        self.uc.emu_start(function, self.stop, count=50_000_000)
        if self.uc.reg_read(UC_ARM_REG_PC) != self.stop:
            raise RuntimeError('Guest instruction budget exhausted')
        return self.uc.reg_read(UC_ARM_REG_R0)

    def transmit(self, frequency, pattern):
        name = self.allocate(16)
        self.uc.mem_write(name, b'transmitter\0')
        device_out = self.allocate(4)
        # Android 32-bit hw_module_t.methods at20; hw_device_t size64.
        method = self.word(self.word(self.hmi+20))
        opened = self.call(method, self.hmi, name, device_out)
        if opened:
            raise RuntimeError(f'HAL open failed: {opened:#x}')
        device = self.word(device_out)
        callback = self.word(device+64)
        durations = self.allocate(4*len(pattern))
        self.uc.mem_write(durations, struct.pack('<'+'I'*len(pattern), *pattern))
        # consumerir_device_t.transmit(dev, frequency, pattern, length).
        result = self.call(callback, device, frequency, durations, len(pattern))
        return {'open_result':opened,'transmit_result':result,'events':self.events}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('hal', type=Path)
    parser.add_argument('--output', type=Path, required=True, help='Private directory for captured bytes/report')
    parser.add_argument('--frequency', type=int, default=38000)
    parser.add_argument('--pattern', default='9000,4500,560,560,560,1690,560', help='Alternating mark/space durations in microseconds')
    parser.add_argument('--mode', choices=['0','1'], default='0')
    args = parser.parse_args()
    pattern = [int(x) for x in args.pattern.split(',')]
    if not pattern or len(pattern)>10000 or any(x<1 or x>1000000 for x in pattern) or not 1000<=args.frequency<=100000:
        parser.error('Invalid bounded frequency/pattern')
    capture = Capture(args.hal, {'irtx.hal.mode':args.mode})
    result = capture.transmit(args.frequency, pattern)
    result.update(hal_sha256=hashlib.sha256(args.hal.read_bytes()).hexdigest(),
                  frequency=args.frequency, pattern_us=pattern,
                  safety='Offline ARM emulation; all I/O simulated; no host or device transmission')
    args.output.mkdir(parents=True, exist_ok=True)
    for index, data in enumerate(capture.writes):
        (args.output/f'write-{index}.bin').write_bytes(data)
    (args.output/'capture.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result,indent=2))


if __name__ == '__main__':
    main()
