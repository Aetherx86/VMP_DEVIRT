## How To Use
#### 1. Compile PinTool
First, we need to compile a PinTool to extract the execution trace and VM context switches from the target binary
```
$ cd deobfuscator
$ make
```
#### 2. Prepare Virtualized Binary
Next, we need to prepare virtualized binary to apply the PinTool
```
$ cd test
$ make
$ (Apply virtualization to the "test_native" binary generated in the previous "make" command using VMProtect)
```
If done correctly, the following results should be produced
```
$ ls
extract_trace.sh
find_regions.sh
Makefile
test.c
test_native
test_native_vmp (or the name of the virtualized binary you specified)
```
#### 3. Find VM Context Switches
Finally, we need to apply the PinTool to the virtualized binary to identify VM context switches
```
$ (In the Makefile, set "TARGET_BINARY" to the virtualized binary)
$ make trace
```
This process will generate the following files

- trace_vmp 
  - execution trace of target binary
  - each row represents [sequence number] [executed instruction][16 general registers][rflags][address to write][address to read]

- vm_enter_vmp
  - sequence number range expected for VM entry
  - instructions that write the 16 general registers and the RFLAGS to memory are included in the range
    
- vm_enter_call_vmp
  - sequence number range expected for VM entry
  - instructions that write the 16 general registers and the RFLAGS to memory are included in the range
  - call instructions are also included
    
- vm_exit_vmp 
  - sequence number range expected for VM exit
  - instructions that read values from memory and load them into the 16 general registers and the RFLAGS are included in the range
    
- vm_exit_ret_vmp
  - sequence number range expected for VM exit
  - instructions that read values from memory and load them into the 16 general registers and the RFLAGS are included in the range
  - ret instructions are also included

## How To Verify
We need to validate whether the tool has correctly identified the VM switches

To do this, rarely used instructions (e.g. SIMD instructinos) can be inserted at the VM entry and exit points
```
int main() {
    ...
    __asm__ __volatile__ ("movaps %xmm0, %xmm0\n");

    // virtualized function
    func();

    __asm__ __volatile__ ("movaps %xmm2, %xmm2\n");
    ...
}
```
In the example above, "movaps" instructions were inserted before and after the virtualized function "func"

And when looking at the "trace_vmp" file which contains the execution trace of the above function, we can consider the point after the "movaps xmm0, xmm0" as the VM Entry
```
170530 movaps xmm0, xmm0;RAX:c;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd110;R8:7fffe45e74c0;R9:0;R10:0;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:206;0;0;;
170531 mov eax, 0x0;RAX:c;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd110;R8:7fffe45e74c0;R9:0;R10:0;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:206;0;0;;
170532 call 0x55555555478a;RAX:0;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd110;R8:7fffe45e74c0;R9:0;R10:0;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:206;7fffffffd108;0;;
170533 push r10;RAX:0;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd108;R8:7fffe45e74c0;R9:0;R10:0;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:206;7fffffffd100;0;;
170534 pushfq ;RAX:0;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd100;R8:7fffe45e74c0;R9:0;R10:0;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:206;7fffffffd0f8;0;;
170535 mov r10, 0xefb4e9a357002440;RAX:0;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd0f8;R8:7fffe45e74c0;R9:0;R10:0;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:206;0;0;;
170536 call 0x5555558dda1a;RAX:0;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd0f8;R8:7fffe45e74c0;R9:0;R10:efb4e9a357002440;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:206;7fffffffd0f0;0;;
170537 sub r10, 0x383908c1;RAX:0;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd0f0;R8:7fffe45e74c0;R9:0;R10:efb4e9a357002440;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:206;0;0;;
170538 mov r10, qword ptr [rsp+0x10];RAX:0;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd0f0;R8:7fffe45e74c0;R9:0;R10:efb4e9a31ec71b7f;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:292;0;7fffffffd100;;
170539 mov qword ptr [rsp+0x10], 0x42268157;RAX:0;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd0f0;R8:7fffe45e74c0;R9:0;R10:0;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:292;7fffffffd100;0;;
170540 push qword ptr [rsp+0x8];RAX:0;RBX:0;RCX:7fffe42c2104;RDX:7fffe459f8c0;RSI:5555559562b0;RDI:1;RBP:7fffffffe120;RSP:7fffffffd0f0;R8:7fffe45e74c0;R9:0;R10:0;R11:555555554a4a;R12:555555554680;R13:7fffffffe200;R14:0;R15:0;rflags:292;7fffffffd0e8;7fffffffd0f8;;
...
```
The tool predicted the VM entry range as follows, with the expected range of 170522-170617 being close to 170530 (movaps)

```
170552-170617
190927-190956
196783-196830
196791-196850
200583-200658
207529-207601
207573-207638
```
This indicates that the identification was fairly accurate
