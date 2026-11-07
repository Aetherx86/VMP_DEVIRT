ROOT_DIR=..
PIN_DIR=$(ROOT_DIR)/pin
TRACER_DIR=$(ROOT_DIR)/deobfuscator/tracer

TARGET_POSTFIX=vmp
TARGET_BINARY=./test_native_$(TARGET_POSTFIX)
TRACE_FILES=trace vm_enter vm_exit vm_enter_call vm_exit_ret

all:
	gcc -O0 -g -o test_native test.c

objdump:
	objdump -S test_native > objdump_test_native
	objdump -S test_vmp > objdump_test_vmp

trace:
	$(PIN_DIR)/pin -t $(TRACER_DIR)/obj-intel64/instracelog.so -- $(TARGET_BINARY)
	$(foreach file, $(TRACE_FILES), mv $(file) $(file)_$(TARGET_POSTFIX); )

trace_debug:
	$(PIN_DIR)/pin -appdebug -t $(TRACER_DIR)/obj-intel64/instracelog.so -- $(TARGET_BINARY)

clean_all: clean
	rm -f test_*

clean:
	rm -f trace* objdump* pin* vm_enter* vm_exit*
	rm -f $(TARGET_BINARY).*
