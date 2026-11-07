/*
 * A pin tool to record all instructions in a binary execution.
 *
 */

#include <stdio.h>
#include <pin.H>
#include <map>
#include <vector>
#include <iostream>
#include <string>

#define WINDOW_SIZE           100

using namespace std;

std::map<ADDRINT, std::string> opcmap;
std::map<ADDRINT, std::string> ins_type;
std::map<ADDRINT, std::string> opbytes;
FILE *fp_trace, *fp_vm_enter, *fp_vm_exit, *fp_vm_enter_call, *fp_vm_exit_ret;

REG regs_idx[16] = {REG_RAX, REG_RBX, REG_RCX, REG_RDX, REG_RSI, REG_RDI, 
                         REG_RBP, REG_RSP, REG_R8,  REG_R9,  REG_R10, REG_R11,
                         REG_R12, REG_R13, REG_R14, REG_R15};

const char *regs_str[16] = {
    "RAX", "RBX", "RCX", "RDX", 
    "RSI", "RDI", "RBP", "RSP", 
    "R8",  "R9",  "R10", "R11", 
    "R12", "R13", "R14", "R15"
};

enum ins_list {
     Ins_Call,
     Ins_Ret,
     None,
};

typedef struct context_node {
     uint64_t seq_num;
     uint64_t general_regs[16];
     uint64_t rflags;
     enum ins_list ins;
} context_node;

vector<context_node> vm_enter_context_list;
vector<context_node> vm_exit_context_list;

uint64_t seq_num;
uint64_t before_general_regs_val[16];
uint64_t before_rflags_val;

uint64_t after_general_regs_val[16];
uint64_t after_rflags_val;

uint64_t addr_to_read;
uint64_t addr_to_write;

void get_context(ADDRINT addr, CONTEXT *fromctx, ADDRINT raddr, ADDRINT waddr) {
     struct context_node context_tmp;

     seq_num++;
     addr_to_write = waddr;
     addr_to_read = raddr;

     for (int i = 0; i < 16; i++) {
          before_general_regs_val[i] = PIN_GetContextReg(fromctx, regs_idx[i]);
     }
     before_rflags_val = PIN_GetContextReg(fromctx, REG_RFLAGS);     

     // add context_node
     context_tmp.seq_num = seq_num;
     for (int  i = 0; i < 16; i++) {
          context_tmp.general_regs[i] = 0;
     }
     context_tmp.rflags = 0;

     if (strcmp(ins_type[addr].c_str(), "call") == 0) {
          context_tmp.ins = Ins_Call;
     } else if (strcmp(ins_type[addr].c_str(), "ret") == 0) {
          context_tmp.ins = Ins_Ret;
     } else {
          context_tmp.ins = None;
     }

     vm_enter_context_list.push_back(context_tmp);
     vm_exit_context_list.push_back(context_tmp);

     // output execution trace
     fprintf(fp_trace, "%ld ", seq_num);
     // fprintf(fp_trace, "%p;", (void *)addr);
     // fprintf(fp_trace, "%s;", opbytes[addr].c_str());
     fprintf(fp_trace, "%s;", opcmap[addr].c_str());

     // for (int i = 0; i < 16; i++) {
     //      fprintf(fp_trace, "%s:%lx;", regs_str[i], before_general_regs_val[i]);
     // }
     // fprintf(fp_trace, "rflags:%lx;", before_rflags_val);
     // fprintf(fp_trace, "write:%lx;", addr_to_write);
     // fprintf(fp_trace, "read:%lx;;", addr_to_read);
     // fprintf(fp_trace, "")

     fprintf(fp_trace, "\n");
}

void trace_vm_enter(ADDRINT addr, CONTEXT *fromctx, ADDRINT raddr, ADDRINT waddr) {
     struct context_node *context_tmp = &vm_enter_context_list.back();

     if (addr_to_write == 0) {
          return;
     }

     uint64_t val_written = *((uint64_t *)addr_to_write);

     for (int i = 0; i < 16; i++) {
          if (before_general_regs_val[i] == val_written) {
               context_tmp->general_regs[i] = 1; 
          } else {
               context_tmp->general_regs[i] = 0;
          }
     }
     if (before_rflags_val == val_written) {
          context_tmp->rflags = 1;
     }
}

void trace_vm_exit(ADDRINT addr, CONTEXT *fromctx, ADDRINT raddr, ADDRINT waddr) {
     struct context_node *context_tmp = &vm_exit_context_list.back();

     if (addr_to_read == 0) {
          return;
     }

     for (int i = 0; i < 16; i++) {
          after_general_regs_val[i] = PIN_GetContextReg(fromctx, regs_idx[i]);
     }
     after_rflags_val = PIN_GetContextReg(fromctx, REG_RFLAGS);     

     uint64_t val_read = *((uint64_t *)addr_to_read);

     for (int i = 0; i < 16; i++) {
          if (after_general_regs_val[i] == val_read) {
               context_tmp->general_regs[i] = 1; 
          } else {
               context_tmp->general_regs[i] = 0;
          }
     }
     if (after_rflags_val == val_read) {
          context_tmp->rflags = 1;
     }
}

static void instruction(INS ins, void *v) {
     ADDRINT addr = INS_Address(ins);
     if (opcmap.find(addr) == opcmap.end()) {
          opcmap.insert(std::pair<ADDRINT, std::string>(addr, INS_Disassemble(ins)));

          if (INS_IsCall(ins)) {
               ins_type.insert(std::pair<ADDRINT, std::string>(addr, "call"));
          } else if (INS_IsRet(ins)) {  
               ins_type.insert(std::pair<ADDRINT, std::string>(addr, "ret"));
          } else {
               ins_type.insert(std::pair<ADDRINT, std::string>(addr, "none"));
          }

          const UINT64 insSize = INS_Size(ins);
          std::string hexBytes;
          for (UINT64 i = 0; i < insSize; i++) {
               unsigned char byteVal = 0;
               PIN_SafeCopy(&byteVal, reinterpret_cast<const void*>(addr + i), 1);

               char temp[4];
               sprintf(temp, "%02x", byteVal);
               hexBytes += temp;
          }
          opbytes.insert(std::make_pair(addr, hexBytes));

     }

     if (INS_IsMemoryRead(ins) && INS_IsMemoryWrite(ins)) {
          INS_InsertCall(ins, IPOINT_BEFORE, (AFUNPTR)get_context, IARG_INST_PTR, IARG_CONST_CONTEXT, IARG_MEMORYREAD_EA, IARG_MEMORYWRITE_EA, IARG_END);
     } else if (INS_IsMemoryRead(ins)) {
          INS_InsertCall(ins, IPOINT_BEFORE, (AFUNPTR)get_context, IARG_INST_PTR, IARG_CONST_CONTEXT, IARG_MEMORYREAD_EA, IARG_ADDRINT, 0, IARG_END);
     } else if (INS_IsMemoryWrite(ins)) {
          INS_InsertCall(ins, IPOINT_BEFORE, (AFUNPTR)get_context, IARG_INST_PTR, IARG_CONST_CONTEXT, IARG_ADDRINT, 0, IARG_MEMORYWRITE_EA, IARG_END);
     } else {
          INS_InsertCall(ins, IPOINT_BEFORE, (AFUNPTR)get_context, IARG_INST_PTR, IARG_CONST_CONTEXT, IARG_ADDRINT, 0, IARG_ADDRINT, 0, IARG_END);
     }
     
     if (INS_IsValidForIpointAfter(ins)) {
          INS_InsertCall(ins, IPOINT_AFTER, (AFUNPTR)trace_vm_enter, IARG_INST_PTR, IARG_CONST_CONTEXT, IARG_END);
          INS_InsertCall(ins, IPOINT_AFTER, (AFUNPTR)trace_vm_exit, IARG_INST_PTR, IARG_CONST_CONTEXT, IARG_END);
     }
}

typedef struct seq_range {
     uint64_t begin;
     uint64_t end;
} seq_range;

static void find_range(FILE *fp, vector<context_node> context_list) {
     uint64_t begin_seq_num, cur_seq_num;
     vector<seq_range> seq_range_list;

     for (auto iter_begin = context_list.begin(); iter_begin != context_list.end(); iter_begin++) {
          uint64_t general_regs_matched[16];
          uint64_t rflags_matched;
          for (int i = 0; i < 16; i++) {
               general_regs_matched[i] = 0;
          }
          rflags_matched = 0;

          begin_seq_num = (*iter_begin).seq_num;

          for (auto iter_end = iter_begin; iter_end != context_list.end(); iter_end++) {
               cur_seq_num = (*iter_end).seq_num;

               // if exceed window_size, begin with next seq_num
               if ((cur_seq_num-begin_seq_num) > WINDOW_SIZE) {
                    break;
               }
               for (int i = 0; i < 16; i++) {
                    if ((*iter_end).general_regs[i]) {
                         general_regs_matched[i] = 1;
                    }
               }
               if ((*iter_end).rflags) {
                    rflags_matched = 1;
               }

               int all_pushed = 1;
               for (int i = 0; i < 16; i++) {
                    if (regs_idx[i] == REG_RSP) {
                         continue;
                    }

                    if (general_regs_matched[i] == 0) {
                         all_pushed = 0;
                         break;
                    }
               }
               if (rflags_matched == 0) {
                    all_pushed = 0;
               }

               // if all registers (except RSP) are in range, record it
               if (all_pushed) {
                    struct seq_range range;
                    range.begin = begin_seq_num;
                    range.end = cur_seq_num;
                    seq_range_list.push_back(range);
                    break;
               }
          }
     }

     if (!seq_range_list.empty()) {
          uint64_t range_begin, range_end;
          auto iter = seq_range_list.begin();

          range_begin = (*iter).begin;
          range_end = (*iter).end;
          iter++;

          for (; iter != seq_range_list.end(); iter++) {
               // if having same range_end, find smallest size of range
               if (range_end == (*iter).end) {
                    range_begin = (*iter).begin;
                    continue;
               }
               fprintf(fp, "%ld-%ld\n", range_begin, range_end);
               range_begin = (*iter).begin;
               range_end = (*iter).end;
          }
          fprintf(fp, "%ld-%ld\n", range_begin, range_end);
     }
}

static void find_vm_enter_range(FILE *fp, vector<context_node> context_list) {
     uint64_t begin_seq_num, cur_seq_num;
     vector<seq_range> seq_range_list;

     fprintf(stderr, "find_vm_enter_range\n");

     for (auto iter_begin = context_list.begin(); iter_begin != context_list.end(); iter_begin++) {
          uint64_t general_regs_matched[16];
          uint64_t rflags_matched;
          for (int i = 0; i < 16; i++) {
               general_regs_matched[i] = 0;
          }
          rflags_matched = 0;

          if ((*iter_begin).ins != Ins_Call) {
               continue;
          }

          begin_seq_num = (*iter_begin).seq_num;

          for (auto iter_end = iter_begin; iter_end != context_list.end(); iter_end++) {
               cur_seq_num = (*iter_end).seq_num;

               // if exceed window_size, begin with next seq_num
               if ((cur_seq_num-begin_seq_num) > WINDOW_SIZE) {
                    break;
               }
               for (int i = 0; i < 16; i++) {
                    if ((*iter_end).general_regs[i]) {
                         general_regs_matched[i] = 1;
                    }
               }
               if ((*iter_end).rflags) {
                    rflags_matched = 1;
               }

               int all_pushed = 1;
               for (int i = 0; i < 16; i++) {
                    if (regs_idx[i] == REG_RSP) {
                         continue;
                    }

                    if (general_regs_matched[i] == 0) {
                         all_pushed = 0;
                         break;
                    }
               }
               if (rflags_matched == 0) {
                    all_pushed = 0;
               }

               // if all registers (except RSP) are in range, record it
               if (all_pushed) {
                    struct seq_range range;
                    range.begin = begin_seq_num;
                    range.end = cur_seq_num;
                    seq_range_list.push_back(range);
                    break;
               }
          }
     }

     if (!seq_range_list.empty()) {
          uint64_t range_begin, range_end;
          auto iter = seq_range_list.begin();

          range_begin = (*iter).begin;
          range_end = (*iter).end;
          iter++;

          for (; iter != seq_range_list.end(); iter++) {
               // if having same range_end, find smallest range
               if (range_end == (*iter).end) {
                    range_begin = (*iter).begin;
                    continue;
               }
               fprintf(fp, "%ld-%ld\n", range_begin, range_end);
               range_begin = (*iter).begin;
               range_end = (*iter).end;
          }
          fprintf(fp, "%ld-%ld\n", range_begin, range_end);
     }
}

static void find_vm_exit_range(FILE *fp, vector<context_node> context_list) {
     uint64_t begin_seq_num, cur_seq_num;
     vector<seq_range> seq_range_list;

     fprintf(stderr, "find_vm_exit_range\n");

     for (auto iter_begin = context_list.begin(); iter_begin != context_list.end(); iter_begin++) {
          uint64_t general_regs_matched[16];
          uint64_t rflags_matched;
          for (int i = 0; i < 16; i++) {
               general_regs_matched[i] = 0;
          }
          rflags_matched = 0;

          begin_seq_num = (*iter_begin).seq_num;

          for (auto iter_end = iter_begin; iter_end != context_list.end(); iter_end++) {
               cur_seq_num = (*iter_end).seq_num;

               // if exceed window_size, begin with next seq_num
               if ((cur_seq_num-begin_seq_num) > WINDOW_SIZE) {
                    break;
               }
               for (int i = 0; i < 16; i++) {
                    if ((*iter_end).general_regs[i]) {
                         general_regs_matched[i] = 1;
                    }
               }
               if ((*iter_end).rflags) {
                    rflags_matched = 1;
               }

               int all_pushed = 1;
               for (int i = 0; i < 16; i++) {
                    if (regs_idx[i] == REG_RSP) {
                         continue;
                    }

                    if (general_regs_matched[i] == 0) {
                         all_pushed = 0;
                         break;
                    }
               }
               if (rflags_matched == 0) {
                    all_pushed = 0;
               }

               // if all registers (except RSP) are in range, record it
               if (all_pushed && (*iter_end).ins == Ins_Ret) {
                    struct seq_range range;
                    range.begin = begin_seq_num;
                    range.end = cur_seq_num;
                    seq_range_list.push_back(range);
                    break;
               }
          }
     }

     if (!seq_range_list.empty()) {
          uint64_t range_begin, range_end;
          auto iter = seq_range_list.begin();

          range_begin = (*iter).begin;
          range_end = (*iter).end;
          iter++;

          for (; iter != seq_range_list.end(); iter++) {
               // if having same range_end, find smallest size of range
               if (range_end == (*iter).end) {
                    range_begin = (*iter).begin;
                    continue;
               }
               fprintf(fp, "%ld-%ld\n", range_begin, range_end);
               range_begin = (*iter).begin;
               range_end = (*iter).end;
          }
          fprintf(fp, "%ld-%ld\n", range_begin, range_end);
     }
}

static void on_fini(INT32 code, void *v) {
     find_range(fp_vm_enter, vm_enter_context_list);
     find_range(fp_vm_exit, vm_exit_context_list);

     find_vm_enter_range(fp_vm_enter_call, vm_enter_context_list);
     find_vm_exit_range(fp_vm_exit_ret, vm_exit_context_list);

     fclose(fp_trace);
     fclose(fp_vm_enter);
     fclose(fp_vm_exit);
     fclose(fp_vm_enter_call);
     fclose(fp_vm_exit_ret);
}

int main(int argc, char *argv[]) {
     if (PIN_Init(argc, argv)) {
          fprintf(stderr, "command line error\n");
          return 1;
     }

     fp_trace = fopen("trace", "w");
     fp_vm_enter = fopen("vm_enter", "w");
     fp_vm_exit = fopen("vm_exit", "w");

     fp_vm_enter_call = fopen("vm_enter_call", "w");
     fp_vm_exit_ret = fopen("vm_exit_ret", "w");

     seq_num = 0;

     PIN_InitSymbols();
     PIN_AddFiniFunction(on_fini, 0);
     INS_AddInstrumentFunction(instruction, NULL);
     PIN_StartProgram(); 

     // Never reached

     return 0; 
}
