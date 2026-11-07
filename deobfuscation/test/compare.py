import re

def load_vm_enter_call_vmp(filename):
    """vm_enter_call_vmp.txt 파일에서 start-end 구간을 리스트로 읽어서 [(start, end), ...] 형태로 반환"""
    ranges = []
    with open(filename, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            # "98730-98816" 꼴 파싱
            start_str, end_str = line.split('-')
            start = int(start_str)
            end = int(end_str)
            ranges.append((start, end))
    return ranges


def load_vm_exit_ret_vmp(filename):
    """vm_exit_ret_vmp.txt 파일에서 start-end 구간을 리스트로 읽어서 [(start, end), ...] 형태로 반환"""
    ranges = []
    with open(filename, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            start_str, end_str = line.split('-')
            start = int(start_str)
            end = int(end_str)
            ranges.append((start, end))
    return ranges


def parse_trace_vmp(filename):
    """
    trace_vmp.txt 파일에서
    <명령어 순서> <명령어>;... 형태를 파싱하여
    [(순서번호, 명령어텍스트), ...] 리스트로 반환
    """
    instructions = []
    pattern = re.compile(r'^(\d+)\s+(.*?)\;')  
    # 예: "181474 movaps xmm0, xmm0;..." -> group(1)="181474", group(2)="movaps xmm0, xmm0"

    with open(filename, 'r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            m = pattern.match(line)
            if m:
                instr_no = int(m.group(1))
                instr_text = m.group(2)
                instructions.append((instr_no, instr_text))
    return instructions


def find_closest_diff_vm_enter(instr_no, vm_enter_ranges):
    """
    instr_no: trace_vmp에서 뽑아낸 명령어 순서(int)
    vm_enter_ranges: [(start, end), ...]
    -> 모든 start에 대해 abs(start - instr_no) 구하고, 최소값 찾기
    반환값: (best_start, min_diff)
    """
    min_diff = None
    best_start = None
    for (start, end) in vm_enter_ranges:
        diff = abs(start - instr_no)
        if min_diff is None or diff < min_diff:
            min_diff = diff
            best_start = start
    return (best_start, min_diff)


def find_closest_diff_vm_exit(instr_no, vm_exit_ranges):
    """
    instr_no: trace_vmp에서 뽑아낸 명령어 순서(int)
    vm_exit_ranges: [(start, end), ...]
    -> 모든 end에 대해 abs(end - instr_no) 구하고, 최소값 찾기
    반환값: (best_end, min_diff)
    """
    min_diff = None
    best_end = None
    for (start, end) in vm_exit_ranges:
        diff = abs(end - instr_no)
        if min_diff is None or diff < min_diff:
            min_diff = diff
            best_end = end
    return (best_end, min_diff)


def main():
    # (예시) 실제 파일 경로를 맞춰서 설정해주세요.
    trace_vmp_file = "trace_vmp"
    vm_enter_file  = "vm_enter_call_vmp"
    vm_exit_file   = "vm_exit_ret_vmp"

    vm_enter_ranges = load_vm_enter_call_vmp(vm_enter_file)
    vm_exit_ranges  = load_vm_exit_ret_vmp(vm_exit_file)
    trace_instructions = parse_trace_vmp(trace_vmp_file)

    # 결과를 담을 리스트 (movaps xmm0, xmm0 / movaps xmm2, xmm2 별도 보관)
    movaps_xmm0_results = []
    movaps_xmm2_results = []

    for (instr_no, instr_text) in trace_instructions:
        lower_text = instr_text.lower()

        # 1) movaps xmm0, xmm0 -> vm_enter_call_vmp의 start와 비교
        if "movaps xmm0, xmm0" in lower_text:
            best_start, min_diff = find_closest_diff_vm_enter(instr_no, vm_enter_ranges)
            movaps_xmm0_results.append((
                "enter",
                instr_no,
                best_start,
                min_diff
            ))

        # 2) movaps xmm2, xmm2 -> vm_exit_ret_vmp의 end와 비교
        elif "movaps xmm2, xmm2" in lower_text:
            best_end, min_diff = find_closest_diff_vm_exit(instr_no, vm_exit_ranges)
            movaps_xmm2_results.append((
                "exit",
                instr_no,
                best_end,
                min_diff
            ))

    # ------------------------
    # 출력 (movaps xmm0, xmm0 부터)
    # ------------------------
    for (mnemonic, instr_no, compare_no, diff) in movaps_xmm0_results:
        print(f"{mnemonic} {instr_no} {compare_no} {diff}")

    # ------------------------
    # 출력 (movaps xmm2, xmm2)
    # ------------------------
    for (mnemonic, instr_no, compare_no, diff) in movaps_xmm2_results:
        print(f"{mnemonic} {instr_no} {compare_no} {diff}")


if __name__ == "__main__":
    main()