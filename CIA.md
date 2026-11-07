# Chosen-Instruction Attack (NDSS 2022) 논문 내용 정리

---

## 핵심 개념 정의

본 논문은 상용 소프트웨어 난독화 도구 중 **가상 기계(VM) 기반 난독화**에 대해 *Chosen-Instruction Attack (CIA)*라는 새로운 공격 기법을 제안한다. 이를 이해하기 위해 먼저 논문에 등장하는 주요 개념들을 정의한다.

### 앵커 명령어 (Anchor Instruction)

**앵커 명령어**란 공격자가 원본 코드에 의도적으로 삽입하는 특수한 명령어로서, 상용 VM 기반 난독화 도구가 해당 명령어를 *가상화하지 않고* 원본 형태 그대로 남겨두는 명령어를 의미한다. 다시 말해, 난독화 도구의 처리 한계나 특성으로 인해 가상화 대상에서 제외되는 명령어가 앵커로 사용된다. 앵커 명령어 자체는 프로그램의 실행에 부작용을 일으키지 않도록 설계되며 (예: 레지스터 자기자신과의 `CMPXCHG EAX, EAX` 연산 등), 프로그램 논리에 영향을 주지 않는 *중립적인 명령어*이다. 이러한 앵커를 코드의 특정 위치에 삽입하면, 난독화된 프로그램을 실행하면서 수집한 *실행 trace* 상에서 해당 명령어를 손쉽게 식별할 수 있다. 결국 앵커 명령어는 긴 실행 trace 중에서 관심 있는 부분(원본 명령어의 가상화된 구현 부분)을 구분해내는 **마커(marker)** 역할을 한다. 논문에서는 수백 개에 달하는 x86 명령어들 중 다양한 상용 난독화기에서 가상화되지 않고 남는 명령어들을 찾아 앵커 후보로 활용하였다.

### 지식 유출 코드 (Knowledge Leaking Code)

**지식 유출 코드**란 공격자가 난독화 도구로부터 “배우고자 하는 지식”을 추출하기 위해 앵커 명령어 사이에 삽입하는 코드 조각을 말한다. 보통 하나 또는 몇 개의 어셈블리 명령으로 구성되며, 이 코드 조각이 바로 공격자가 분석하고 싶은 대상이 되는 *원본 명령어*이다. 예를 들어, 공격자가 특정 산술 연산 명령어의 가상화 구현 방식을 알고 싶다면 해당 명령어를 지식 유출 코드로 삽입한다. 이렇게 삽입된 지식 유출 코드는 난독화 도구에 의해 가상화된 후 매우 복잡한 형태의 가상 기계 코드로 변환되는데, CIA 공격자는 그 결과를 분석하여 원본 명령어와 가상 코드 사이의 **매핑 규칙(mapping rule)**이나 가상화 기법에 관한 지식을 얻어낸다. 요약하면, 지식 유출 코드는 *공격자가 의도적으로 선택(chosen)한 원본 명령어 또는 코드 조각*이며, 이 코드의 가상화 결과를 통해 난독화기의 내부 동작 지식을 “유출”시키는 역할을 한다.

### 가상 머신 구조 (Virtual Machine Architecture)

VM 기반 난독화에서 말하는 **가상 머신(VM) 구조**란 난독화 도구가 원본 프로그램을 변환하여 만들어내는 *가상화된 실행 환경과 그 구성 요소들*을 뜻한다. 일반적으로 상용 난독화 도구는 원본 프로그램의 일부 명령어들을 가상 기계 코드로 대체하는데, 이때 생성되는 가상 기계는 다음과 같은 구조적 요소를 가진다:

- **컨텍스트 스위치(Context Switch)**:
  가상 기계 실행을 시작하기 전에 원본 프로그램의 레지스터 등 실행 상태를 별도의 *가상 컨텍스트*로 저장하고, 가상 기계 실행이 끝난 후 원복하는 과정이다. 보통 VM의 시작 지점에서 원본 레지스터 값들을 백업하고 가상 레지스터로 옮긴 뒤, 종료 시 계산 결과를 다시 원본 레지스터에 복원하는 명령어 시퀀스로 구현된다. 이 컨텍스트 저장/복원 코드가 원본 코드 영역과 가상 코드 영역의 경계를 이루므로, 많은 분석 기법에서 *가상화 코드의 시작과 끝을 식별*하는 단서로 활용된다.

- **디스패처(Dispatcher)**:
  가상화된 프로그램 내에서 가상 명령어(바이트코드)를 읽어들여 해당 명령어를 처리하는 *가상 핸들러(handler)*를 호출하는 제어 구조이다. 전통적인 인터프리터 형태의 VM에서는 보통 `while` 루프를 돌며 바이트코드를 fetch & decode하고, 해석된 명령 타입에 따라 각 핸들러를 호출(분기)하는 *디코드-디스패치 루프* 구조를 가진다. 하지만 이러한 중앙 루프 형태는 패턴이 눈에 띄어 분석에 취약하므로, 최신 상용 난독화기들은 각 가상 명령 핸들러 끝에 다음 핸들러로 직행하는 분기(*direct threading*) 기법을 써서 명시적인 루프 없이 디스패처 기능을 분산시키기도 한다.

- **가상 명령어 및 핸들러들**:
  난독화기가 원본 프로그램의 명령어를 대체하여 생성하는 일련의 *가상 명령(바이트코드)*들과, 그 각각을 실행하기 위한 코드 조각이 *가상 핸들러*이다. 난독화기는 내부적으로 정의한 매핑 규칙에 따라 각 원본 명령어를 하나 혹은 여러 개의 가상 명령어 시퀀스로 변환한다. 가상 핸들러는 이러한 가상 명령들의 동작을 수행하는 기계어 코드로, 원본 명령어와 등가의 기능을 (종종 훨씬 복잡한 방법으로) 구현한다. 고급 난독화기의 가상 핸들러들은 역공학을 어렵게 하기 위해 의미 없는 부가 연산(*junk code*)이나 다양한 난독화 기법(명령어 난수화, 암호화, 메타모르픽 변환 등)이 적용되어 실제 동작을 은닉한다.

이러한 VM 구조 전체가 원본 프로그램 내에 삽입되어 실행 시간에 원본 프로그램의 일부를 에뮬레이션하게 된다. 상용 난독화기들은 분석 회피를 위해 바이트코드 암호화, 가짜 디스패처 루프 삽입, VM 구성요소의 무작위 배치 등 여러 기법을 통합하여 VM 구조를 복잡하게 만든다. 또한 일부 제품은 여러 종류의 VM 설정을 제공하여 난독화 다양성을 높인다 (예: Themida/Code Virtualizer는 서로 다른 6가지 VM 프로파일을 제공하며, VMProtect도 여러 버전에서 다른 매핑 규칙을 사용). 이러한 VM 구조의 복잡성은 곧 난독화 강도와 직결되며, 분석가가 가상화된 코드를 풀어내기 어렵게 만드는 주요 원인이다.

---

## 연구 배경과 문제 정의

코드 가상화 기반 난독화는 현재까지 알려진 난독화 기술 중에서도 **가장 복잡하고 강력한 기법**에 속한다~\cite{vmpsoft}. 상용 난독화 도구(예: Themida, VMProtect 등)는 이러한 VM 기반 난독화를 사용하여 프로그램의 일부를 자체 정의한 가상 기계 코드로 변환함으로써, 분석자가 원본 프로그램의 동작을 직접 추적하기 매우 어렵게 만든다. 특히 이들 도구는 주로 악성코드 개발자들이 악성 행위를 숨기기 위해 악용하고 있으며, 가상화된 악성코드는 실행 시에만 원본 동작이 드러나고 정적인 분석이나 단순 디버깅으로는 그 내부 동작을 이해하기 힘들다.

문제는, 상용 난독화기의 **내부 구조와 매핑 규칙이 모두 비공개(black-box)**라는 점이다. 이러한 도구가 입력 프로그램의 명령어를 어떻게 변환하는지, 가상 머신을 어떻게 구성하는지는 제조사만 알 뿐 분석가에게는 알려져 있지 않다. 결국 보안 분석가나 역공학자는 *난독화된 바이너리(O(P)) 자체를 역공학*하여, 그 안에 숨겨진 VM 구조와 매핑 규칙 등의 “지식”을 찾아내야 한다. 그러나 상용 난독화기가 만들어내는 코드 규모는 방대하고 복잡도도 높아서 (예: 수십만 개 이상의 가상 명령 실행, 암호화된 바이트코드, 난수화된 제어 흐름 등), 인간이 직접 이를 분석하는 과정은 막대한 시간과 노력을 요구하며 부정확성이 크다. 요컨대, **VM 기반 난독화에서 핵심 난제는 “가상화된 코드 조각 속에서 의미있는 핵심 명령들을 식별하고 원본 코드와의 대응 관계를 알아내는 것”**이다. 이를 위한 지식을 얻으려면 결국 반복적인 추론과 시행착오를 거쳐 난독화기의 동작 원리를 역추적해야 하는데, 이 작업은 *매우 비용이 크고 오류를 수반하는 과정*이라는 것이 연구 배경에서 지적된다.

이러한 상황에서 기존 연구들은 난독화된 코드에서 패턴을 찾아 **VM 구조를 탐지**하거나, 원본-가상 코드 간의 일부 **매핑 관계를 밝혀내어 역난독화(deobfuscation)**를 시도해 왔다. 하지만 상용 난독화기의 완벽한 블랙박스 특성과 공격자와의 *지식 격차* 때문에, 기존 접근법들은 제한적인 휴리스틱이나 수동 분석에 의존하는 경우가 많았다. 본 논문은 이러한 문제를 해결하기 위해 **공격자가 난독화기에 능동적으로 대응하여 지식을 추출하는 새로운 모델(CIA)를 제안**하고, 이를 통해 난독화 내부 지식을 자동으로 획득함으로써 향후 난독화 분석 및 우회 기법 개발을 크게 향상시킬 수 있음을 보여준다.

---

## 기존 연구들과의 차이점

기존의 VM 난독화 분석 연구들은 주로 난독화된 결과물(O(P))을 입력받아, 그 안에서 특정 패턴이나 알려진 특징을 찾아내는 **수동적 분석(passive analysis)**에 집중해 왔다. 예를 들어, VMHunt~\cite{salem2014protecting}와 같은 도구는 실행 trace에서 *컨텍스트 스위치* 명령 패턴을 찾아 가상화된 코드 영역을 식별하고자 했고, 다른 연구들에서는 *디스패처 루프의 존재*나 함수 호출/리턴 패턴을 단서로 가상 핸들러들의 경계를 찾으려는 시도가 있었다. 또한 Syntia~\cite{lesaint2019syntia} 등의 기법은 난독화된 코드 조각의 입출력 동작을 관찰하여 원본 의미를 추측(프로그램 신텟시스)함으로써 역난독화를 시도하기도 했다. 이러한 선행 연구들은 난독화기의 일부 동작을 해석하거나 추론하는 데 성공을 거두었으나, 다음과 같은 한계를 지닌다:

- **지식 획득의 불완전성**:
  기존 방법들은 난독화된 코드에서 사전에 알려진 패턴만을 찾거나, 제한된 범위의 명령어에 대해서만 매핑을 유추할 수 있었다. 예를 들어, 단순한 디스패처 루프를 탐지하는 방법은 Themida처럼 고급 기법(직접 스레딩, 가짜 루프 삽입 등)으로 루프를 숨기는 경우 실패한다. 또한 원본-가상 명령 매핑을 인간 전문가의 추론에 의존하거나 일부 알려진 VM ISA에만 적용하는 등 *자동화와 일반화*가 어려웠다.

- **오탐 및 미탐지 문제**:
  난독화 코드에는 의미 없는 *junk 코드*가 다량 포함되어 있어서, 단순 패턴 매칭으로는 진짜 의미있는 연산과 구분이 힘들다. 이로 인해 기존 기법들은 가상 핸들러에서 핵심 연산을 누락하거나 반대로 junk 코드를 잘못 의미있는 코드로 식별하는 등 정밀도에 한계가 있었다. 논문에서도 기존의 세 가지 대표적 역난독화 기법(VMHunt, “generic deobfuscator”~\cite{yadegari2015generic}, Syntia)이 체계적인 평가 없이 제안되어, 실제 상용 난독화에 적용하면 *부정확한 결과를 초래함*을 지적하고 있다.

- **사전 지식 필요**:
  무엇보다, 과거 연구들은 난독화기의 내부 작동에 대한 연구자 자신의 *사전 지식*이나 수작업 분석의 결과에 크게 의존했다. 예컨대, 어떤 연구는 특정 상용 난독화기에서 추출한 가상 ISA 명령 목록을 가정하고, 다른 연구는 사람이 식별한 매핑 규칙을 활용하는 등 일반화된 자동 기법이라기보다 사례별 공략에 가까운 것이 현실이었다.

이에 비해 **Chosen-Instruction Attack (CIA)** 기법은 *공격자 관점에서 난독화기에 능동적으로 상호작용*함으로써, 이전 기법들의 한계를 넘어서는 보다 일반적이고 자동화된 지식 획득 방법을 제시한다. 주요 차별점은 다음과 같다:

- **능동적 공격 모델**:
  CIA는 분석 대상 난독화기를 블랙박스로 두고 일방적으로 역공학하는 대신, *공격자가 임의의 프로그램을 만들어 난독화기에 넣어보고 그 결과를 관찰*하는 **chosen-plaintext** 스타일의 접근을 취한다. 이를 통해 분석자는 난독화기의 동작을 탐색하는 실험을 직접 설계할 수 있고, 다양한 입력 조합을 시도하여 포괄적인 지식을 습득할 수 있다. 이는 기존의 수동 분석보다 훨씬 *능동적이고 체계적인* 방법론이다.

- **앵커 명령어 도입으로 효율적인 추적**:
  긴 실행 trace에서 유의미한 부분만을 발췌하는 **앵커 개념**을 도입하여, 불필요한 분석 비용을 크게 줄였다. 앵커를 활용하면 관심 있는 지식 유출 코드 주변의 가상화된 명령어만을 빠르게 찾아낼 수 있으므로, 기존 기법들처럼 수만\textasciitilde~수십만 개의 명령 trace 전체를 분석하지 않아도 된다. 이는 *잡음 감소* 측면에서 중요한 개선이다.

- **Guided Simplification 기법**:
  CIA에서는 단순 패턴이나 휴리스틱 대신, **원본 명령어의 의미(정의된 동작)를 활용하여** 가상화된 명령 시퀀스를 정밀 추출하는 *Guided Simplification* 알고리즘을 제안했다. 이 방법은 trace 내의 각 명령어가 최종 결과에 끼치는 영향도를 고려한 *프로그램 슬라이싱*과 명령어 효과 *정규화*를 통해, junk 코드를 걸러내고 핵심 동작을 구현하는 명령만 추출한다. 또한, SMT 기반 *기호 실행(symbolic execution)*을 통해 추출한 명령 시퀀스의 정확성을 검증함으로써 오탐을 줄였다. 이러한 자동화된 정밀 분해 기법은 기존의 수작업/휴리스틱 접근과 확연히 구별되는 CIA만의 특징이다.

- **체계적인 대규모 평가와 지표**:
  논문에서는 CIA를 통해 추출한 지식을 활용하여 **처음으로 상용 난독화에 대한 벤치마크 데이터셋**을 구축하고, 이를 이용해 기존 역난독화 기법들의 성능을 정량적으로 평가하였다. 이를 통해 과거 기법들의 문제점을 구체적으로 드러내고 향후 개선 방향을 제시한 점에서, CIA는 단순한 공격 기법 제안에 그치지 않고 *난독화 연구 분야의 평가 방법론을 한 단계 발전*시켰다.

요약하면, CIA는 기존 연구의 경험적 한계를 인지하고 이를 넘어 **자동화된 지식 추출과 활용**을 가능케 한 새로운 접근법으로서, VM 난독화 분석 분야에 중요한 기여를 하고 있다.

---

## 제안 기법: Chosen-Instruction Attack (CIA)

이 절에서는 본 논문의 핵심인 **Chosen-Instruction Attack (CIA)** 기법을 자세히 설명한다. CIA는 암호학의 *선택 평문 공격(chosen-plaintext attack)*에서 착안한 것으로, 공격자가 난독화기에 여러 입력을 시도해 봄으로써 내부 정보를 역추론하는 모델이다. 여기서는 공격자(**A**)와 난독화기(**O**)의 상호작용을 가정한다. 공격자 A는 난독화기 O에 대해 자유롭게 프로그램을 공급할 수 있고, O는 그 프로그램을 난독화하여 출력한다 (상용 난독화 도구를 입수하여 사용하는 시나리오를 가정). A의 목표는 O에 내재된 가상화 지식 $k$ (예: VM 구조, 명령 매핑 규칙 등)를 *학습하고 추출*하는 것이다. 이를 위해 A는 여러 번에 걸쳐 O에 *지식 유출 프로그램*을 입력하고, 각 출력 프로그램 $O(P)$을 분석하여 지식 $k$를 축적하게 된다. CIA의 전체 절차를 단계별로 정리하면 다음과 같다:

### 앵커 명령어 삽입과 가상화 프로그램 생성

먼저 공격자 A는 난독화기 O로부터 추출하고자 하는 특정 지식에 대응하는 **원본 코드 조각 $c$**을 선택한다。$c$는 보통 하나의 어셈블리 명령어나 짧은 명령어 시퀀스로, 앞서 정의한 *지식 유출 코드*에 해당한다. 이 $c$를 통해 A는 해당 명령어의 가상화 구현 방식을 배우고자 한다. 다음으로 A는 $c$를 포함하는 간단한 입력 프로그램 $P_c$를 작성한다. 이때 $c$의 앞뒤에는 **앵커 명령어**를 각각 한 개씩 삽입하여, $P_c$ 내에 “[앵커]-[$c$]-[앵커]” 패턴을 구성한다。또한 난독화기 O가 제공하는 *가상화 대상 지정 매크로* 등을 사용하여 $P_c$ 중 $c$ 부분만이 가상화되도록 설정한다。\footnote{실제로 Themida, Code Virtualizer, VMProtect 등 상용 도구들은 개발자에게 특정 함수나 블록을 가상화하도록 지정하는 매크로나 pragma를 제공한다. 논문에서는 이러한 매크로(`VIRTUALIZER_START`, `VMProtectBegin` 등)를 활용하여 지식 유출 코드 부분만 가상화하도록 한다。} $P_c$의 나머지 부분(앵커 및 기타 코드)은 난독화되지 않은 채로 남도록 함으로써, $c$ 부분의 가상화 결과를 명확히 분리할 수 있다。

이 과정을 거쳐 구성된 입력 프로그램 $P_c$의 한 예를 코드로 나타내면 다음과 같다：

```c
    // Example function for knowledge leaking (using CMPXCHG as anchor, XOR as leaking code)
    void KnowledgeLeaking(){
        VIRTUALIZER_START    // Virtualization start macro (e.g., VMProtectBegin)
        __asm(
            "cmpxchg eax, eax;"   // Anchor instruction (not virtualized)
            "xor ebx, 0xDEAD;"    // Knowledge leaking code (virtualized instruction)
            "cmpxchg eax, eax;"   // Anchor instruction (not virtualized)
        );
        VIRTUALIZER_END     // Virtualization end macro (e.g., VMProtectEnd)
    }
```

위 코드에서 `cmpxchg eax, eax` 명령어는 대표적인 앵커 예시로, 어떤 난독화기에서도 가상화되지 않고 원본 형태로 남는 명령어다. 중간의 `xor ebx, 0xDEAD`는 가상화되어 복잡한 가상 코드로 변환될 명령어로, 이처럼 임의의 명령어로 대체하여 다양한 지식을 추출할 수 있다。

이제 A는 이렇게 만든 입력 프로그램 $P_c$를 난독화기 O에 넣어 가상화된 출력 프로그램 $O(P_c)$를 얻는다。이 $O(P_c)$에는 $c$에 해당하는 부분이 O의 내부 매핑 규칙에 따라 가상 명령어들의 집합으로 변환되어 들어있을 것이다. (예를 들어, VMProtect에 $P_c$를 넣었다면 `xor ebx, 0xDEAD` 원본 명령어가 해당 VMProtect 버전의 가상 명령 시퀀스로 바뀌어 들어있게 된다。) 공격자 A는 이렇게 **여러 종류의 $c$**로 $P_c$를 만들어 가며 O를 반복 호출함으로써, O가 다양한 명령어들을 어떻게 처리하는지 사례를 수집하게 된다。

### 동적 실행 추적 수집 및 앵커 기반 분석

난독화된 프로그램 $O(P_c)$를 입수한 공격자는, 이제 그 내부의 가상화 구현을 분석하기 위해 **실행 추적(trace)**을 활용한다。$O(P_c)$를 실제로 실행하면서 명령어 단위로 추적하면, 가상화된 명령어들이 수행되는 순서를 관찰할 수 있다. 하지만 일반적으로 $O(P_c)$ 전체의 trace는 매우 길고 복잡할 수 있다. 여기서 앞서 삽입한 *앵커 명령어*가 큰 역할을 한다. $O(P_c)$를 실행할 때 앵커 명령어는 난독화되지 않은 채로 등장하므로, 실행 trace 상에서도 명확히 식별된다。공격자 A는 이 trace에서 **두 개의 앵커 명령어 사이에 실행된 명령어들만 추출**함으로써, $c$의 가상화 구현에 직접 관련된 부분만을 분리해낼 수 있다. 이는 앵커 이전이나 이후에 나타나는 난독화되지 않은 코드나, 혹은 가상화된 코드 중 $c$와 무관한 다른 부수적인 동작들을 배제하는 효과가 있다。

보다 구체적으로, A는 $O(P_c)$를 실행하면서 **동적 분석 도구** (논문 구현에서는 Intel PIN 기반의 *동적 계측기* 사용)를 이용하여 명령어 실행을 기록한다。이 계측기는 실행 중 등장하는 `cmpxchg eax, eax` (앵커) 명령어를 감지하면 추적을 시작하거나 끝내는 식으로 동작하여, 결과적으로 *앵커와 앵커 사이의 실행된 명령어 목록*을 출력한다。예를 들어 위 코드의 경우 첫 번째 앵커 이후부터 두 번째 앵커 직전까지 실행된 명령어들이 $c$의 가상화 코드일 것이므로, 그 구간만 별도로 떼어내는 것이다. 이렇게 하면 전체 프로그램의 실행 흐름에서 $c$와 관련 없는 부분 (예: 프로그램 초기화나 기타 함수 실행 등)을 무시할 수 있어 **trace의 길이를 획기적으로 줄이고 분석 집중도를 높인다**。 실제 논문 결과에 따르면, 이러한 앵커 활용으로 얻는 trace는 기존에 비해 훨씬 *짧고 핵심적인 명령어들*로 이루어져 있어 후속 처리 비용을 절감할 수 있었다. 또한 동적 계측 단계에서 앵커 명령어 자체는 별 의미가 없으므로 trace에서 제거하거나 무시한다 (앵커는 오직 경계 표시 용도이므로, 분석 단계에서 이를 제외하여 오염을 막는다)。

### Guided Simplification을 통한 가상 코드 정밀 추출

앵커를 이용해 $c$와 관련된 가상 명령어들의 trace 조각을 얻었다고 해도, 그 내부에는 여전히 *가상화기가 부가적으로 삽입한 불필요한 연산들(junk code)*이 포함되어 있을 수 있다。상용 난독화기들은 분석을 어렵게 하기 위해 실제 의미와 관계없는 연산이나 난독화를 많이 끼워넣기 때문에, 추출한 trace 조각에는 원본 명령어 $c$의 핵심 동작 이외의 것들이 많이 섞여 있다. 따라서 다음 단계는 이 추출한 명령어 시퀀스에서 **진짜 의미있는 부분만 남기는 것**, 즉 *단순화(simplification)* 작업이다。논문에서는 이를 **Guided Simplification(지도된 단순화)** 기법으로 명명하고 구체적인 알고리즘(Algorithm 1)으로 제시하고 있다。

Guided Simplification의 핵심 아이디어는, *원본 명령어 $c$의 수행 결과를 기준으로 가상 명령들의 유의미성을 판단*하는 데 있다。$c$가 원래 프로그램 상태(레지스터나 메모리)에 미치는 영향이 무엇인지를 알고 있으므로, trace 내의 명령들 중 그 영향(데이터 흐름 경로)에 직접 기여하지 않는 명령은 제거하는 방식이다。구체적으로, 알고리즘은 수집된 trace 조각을 **역방향 슬라이싱(backward slicing)**한다: $c$의 결과가 반영되는 최종 목적지 (예: 연산 결과가 저장되는 레지스터)로부터 거슬러 올라가며, 그 값을 계산하는데 필요하지 않은 명령어들은 제외한다. 이렇게 하면 junk 연산이나 단순 난독화용 연산(예: 난수 발생, 더미 연산 등)들은 결과에 영향이 없으므로 걸러지게 되고, 실제 $c$의 기능을 실현하는데 관여한 연산들만 남게 된다. 또한 단순화 과정에서 난독화기마다 상이한 표현으로 구현했을 수 있는 연산들을 정규화(normalization)하여, 본질적으로 동일한 역할을 하는 명령어들은 일반화된 형태로 비교/대비한다。(예를 들어, 어떤 난독화기는 XOR 연산을 여러 번의 NAND/NOR 조합으로 구현했을 수 있는데, 이를 수학적으로 정리하면 결국 XOR와 등가임을 확인하고 표현을 단순화할 수 있다。)

이렇게 Guided Simplification을 적용하면, 원본 명령어 $c$의 의미를 구현하는 **핵심 가상 명령어 시퀀스 $S$**를 추출할 수 있다. 마지막으로 남은 과제는 이 추출된 $S$가 정말로 $c$와 동일한 동작을 하는지 확인하는 것이다. 논문은 이를 위해 **기호 실행 및 공식 검증**을 사용했다. 구체적으로, $S$와 $c$ 각각을 *기호 실행 엔진* (구현에서는 `angr` 프레임워크와 Z3 SMT 솔버 활용)을 통해 수학적인 *정수 논리식(formula)*로 표현한 후, 두 식이 동치임을 체크한다。만약 동치로 판명되면 $S$는 $c$의 정확한 가상 구현으로 *검증(validated)*되며, 공격자는 이 $S$를 **매핑 규칙**으로서 지식 DB에 저장한다. 만약 일치하지 않는다면 (예: 단순화 과정에서 핵심 명령을 잘못 날려버렸거나, trace 수집이 불완전한 경우 등) 해당 결과는 폐기하거나 알고리즘의 매개변수를 조정하여 재시도하게 된다。

이상의 과정을 통해, 공격자 A는 특정 원본 명령어 $c$에 대응하는 **가상화 명령어 시퀀스($S$)와 매핑 규칙**을 획득하게 된다. CIA 공격자는 이러한 절차를 *다양한 명령어 $c$에 대해 반복*함으로써 난독화기 O의 광범위한 명령 커버리지에 대한 지식을 축적할 수 있다. 예를 들어 산술 연산, 논리 연산, 메모리 연산, 제어 흐름 명령 등 카테고리별로 $c$를 선정하여 돌려보면, O가 지원하는 거의 대부분의 명령어에 대한 매핑 규칙을 얻어낼 수 있다. 또한 한 가지 명령어라도 난독화기가 여러 가지 형태로 가상화할 수 있기 때문에 (예: VMProtect는 XOR를 한 가지 방법이 아니라 NAND/NOR 조합 등 여러 방식으로 가상화할 수 있음), 동일한 $c$를 여러 번 난독화해서 서로 다른 $S$가 나오는지도 확인함으로써 **다대일/일대다 매핑 규칙**도 파악할 수 있다。 (논문에서는 이러한 매핑 패턴을 *One-to-One (O2O)*, *One-to-Multiple (O2M)*, *Multiple-to-One (M2O)* 등으로 분류하여, 상용 난독화기들이 각기 어떤 전략을 쓰는지 분석하였다。)

요약하면, Chosen-Instruction Attack (CIA)은
(1) 앵커와 지식 유출 코드를 활용한 *지식 추출용 입력 프로그램 생성*,
(2) 난독화된 출력 실행과 앵커 기반 *핵심 trace 추출*,
(3) Guided Simplification을 통한 *정밀 명령 추출 및 검증*이라는 일련의 과정을 통해, **상용 VM 난독화기의 내부 지식(매핑 규칙, VM 구조 요소 등)을 자동으로 학습**하는 기법이다。 이러한 과정을 거치면 공격자는 얻은 지식을 기반으로 난독화된 프로그램을 더 잘 이해하거나, 새로운 프로그램이 같은 난독화기로 보호되었을 때 빠르게 해독하는 등의 응용이 가능하다. 다음 절에서는 실제 상용 난독화 도구들을 대상으로 CIA를 수행한 결과와 그를 통한 분석 내용을 살펴본다。

## 실험 및 결과 분석

본 논문의 저자들은 제안한 CIA 기법의 효과와 추출된 지식의 활용성을 평가하기 위해, 대표적인 상용 VM 기반 난독화 도구 4종에 CIA를 적용하였다。실험 대상은 **VMProtect** (v2, v3), **Code Virtualizer** (Themida의 OEM, v2, v3), **Themida** (v2, v3), **Obsidium** (v1.6x)으로, 악성코드나 상업 소프트웨어 보호에 널리 쓰이는 제품들이다. 각 도구별로 다양한 옵션 (예: Themida는 6가지 VM 타입과 3가지 난독화 강도 조합 옵션이 있음)을 조합하여 다수의 난독화 결과를 실험하였다。평가 목표는 크게 세 가지 RQ(Research Question)로 정리된다：

- **RQ1: 현실의 상용 난독화 도구들로부터 얼마나 많은 *앵커 명령어*를 찾을 수 있는가?**
- **RQ2: CIA 기법으로 이들 난독화 도구로부터 어떤 종류의 *지식*을 추출할 수 있는가?**
- **RQ3: 이렇게 추출한 지식이 기존 역난독화 기법의 한계를 극복하거나 개선하는 데 도움이 되는가?**

### RQ1: 사용 가능한 앵커 명령어의 수

먼저, 각 난독화기에서 통용될 수 있는 앵커 명령어를 찾아낸 결과, **x86 ISA 총 1584개 명령어 중 760개 (약 48\%)**에 달하는 명령어들이 앵커로 활용 가능함을 확인하였다。이들은 난독화기별로 차이가 있지만, 전반적으로 부동소수점 연산(FPU 명령)과 SIMD(예: SSE/AVX) 명령 대부분이 가상화되지 않고 원본 그대로 남는 것으로 나타났다. 예를 들어 VMProtect v3의 경우 16비트 BCD 연산명령(AAA, AAS 등)이나 `BOUND` 같은 드문 명령어들이 앵커로 발견되었고, Themida 계열의 VM들은 x87 FPU 명령 등 복잡한 부동소수점 연산을 아예 지원하지 않아 모두 앵커가 되었다。반면 일반적인 산술/논리 명령 대다수는 가상화되지만, 그중에서도 `CMPXCHG EAX, EAX`처럼 기능적으로 noop에 가까운 일부 특수 명령은 모든 난독화기에서 가상화하지 않고 그냥 둔다. 또한 Obsidium은 지원하는 명령 종류가 비교적 제한적이어서, 특히 8비트/16비트 피연산을 갖는 명령이나 일부 잘 쓰이지 않는 명령들이 가상화 불가능하여 앵커가 되는 경우가 많았다. 이처럼 **공격자는 다양한 앵커 명령어를 확보할 수 있으며**, 경우에 따라 한 난독화기에서 찾은 앵커를 다른 난독화기에 사용할 수도 있다. (논문 부록에는 인텔 명령어 세트 전체에 대해 어떤 명령어가 어떤 난독화기에서 앵커로 쓰일 수 있는지 표로 정리되어 있다。) 결론적으로 RQ1에 답하자면, 실제 상용 난독화기들에는 수백 개에 달하는 앵커 명령어가 존재하며, 이는 CIA 공격을 다양한 환경에서 성능 좋게 수행할 수 있는 기반이 된다。

### RQ2: 추출된 지식의 유형과 범위

CIA를 통해 네 가지 난독화기들로부터 추출한 지식은 크게 두 부류로 요약된다: **(a) 명령 매핑 규칙의 집합**과 **(b) VM 구조에 대한 정보**이다。

먼저 (a) 매핑 규칙과 관련해서, 연구진은 총 **1915개의 가상 명령 매핑 규칙**을 추출하고 이를 검증하였다。여기에는 다양한 원본 명령어 (산술, 비트연산, 메모리 접근, 제어 흐름 등)에 대한 가상화 구현들이 포함된다。한 가지 원본 명령어에 대해서도 난독화기별로 서로 다른 구현이 나오므로, 서로 다른 규칙들로 집계되었다. 예를 들어, `ADD` 연산 하나도 VMProtect, Themida, Obsidium 등이 각기 다른 가상 명령 조합으로 구현하였기에 별도의 규칙으로 기록된다. 또한 VMProtect와 같이 동일 명령을 여러 방식으로 가상화하는 경우 (앞서 언급한 O2M 형태), XOR에 대해 3가지 변형 구현 규칙을 얻는 등 하나의 원본에 다수 규칙이 실제 도출되기도 했다. 이러한 매핑 지식으로부터, 저자들은 각 난독화기의 **매핑 변환 전략**을 분석하였다. 예컨대 VMProtect는 일부 산술/논리 연산을 한 번에 가상화하지 않고 여러 단계의 가상 명령으로 쪼개 구현(O2M)하거나, 반대로 Code Virtualizer는 간혹 두 개 이상의 원본 연산을 하나의 가상 명령으로 통합(M2O)하기도 함을 확인하였다。CIA를 통해 공격자는 이처럼 *난독화기의 내부 명령어 처리 로직을 상세히 밝혀내는 것*이 가능하다。

두 번째로 (b) VM 구조 정보와 관련하여, CIA는 **컨텍스트 스위치 명령 패턴**과 **디스패처 구조 유형** 역시 추출/분석하였다. 예를 들어, 공격자가 지식 유출 코드로 단순 `NOP` 명령을 사용하면 난독화기 O는 실제 연산이 없는 순수한 VM 진입/종료 코드만 생성하게 된다. 이를 CIA로 분석하면 O가 가상 레지스터 세트를 어떻게 원본 레지스터와 교환하는지, 즉 컨텍스트 스위치 부분의 구현을 정확히 추출할 수 있었다. 그 결과, Code Virtualizer와 Themida의 VM들은 진입 시 특정 레지스터들을 스택에 백업하고 플래그 레지스터도 별도로 저장하는 공통 패턴이 있음을 확인했고, VMProtect는 스레드 로컬 저장소를 이용한 컨텍스트 보관 등 서로 다른 방식을 쓰는 등 **제품별 컨텍스트 스위치 양상**을 파악하였다。더 나아가, 공격자 A는 앵커 명령어를 VM 코드 외부에도 넣어서 (즉 가상화 영역 전후뿐 아니라 프로그램 전체 흐름에서) 난독화기가 삽입한 기타 특수 코드 조각들을 탐지해내는 응용도 가능하다. 연구진은 이러한 방법으로 Themida 계열 난독화기가 가짜 컨텍스트 스위치나 가짜 디스패처 루프를 의도적으로 삽입하는 기법을 썼음을 알아냈다. 디스패처 구조에 대해서는, 각 난독화기의 VM이 *전통적인 중앙 디스패처 루프*를 쓰는지 또는 *직접 스레딩(direct-threaded)*을 쓰는지 분류하였다. VMProtect와 Obsidium은 루프형 디스패처를 사용하고 있어 trace 상에 고정된 루프 패턴이 나타난 반면, Themida/CodeVirtualizer는 가상 핸들러 내에 다음 주소로 바로 뛰는 분기들을 심어 루프를 숨기는 구조임을 확인하였다. 이러한 VM 구조 지식은, 향후 역난독화 도구가 해당 난독화기를 상대할 때 어떤 탐지 전략을 써야 할지 결정하는데 도움을 준다。

정리하면, RQ2에 대해 CIA를 통해 **명령 매핑 규칙 1915개와 VM 구조 관련 지식 (컨텍스트 스위치 구현, 디스패처 유형 등)**을 다수 획득하였으며, 이는 상용 난독화기의 내부 메커니즘을 상당 부분 명백히 밝히는 성과라고 할 수 있다。

### RQ3: 추출 지식의 활용 및 기존 기법 평가

연구진은 CIA로부터 얻은 지식을 **벤치마크 데이터셋**과 **역난독화 기법 평가**에 활용하였다。우선, 추출한 매핑 규칙들을 조합하여 총 **5745개의 가상화된 프로그램**으로 이루어진 최초의 *정교한(fine-grained) 벤치마크*를 구축했다. 이 데이터셋은 다양한 난독화 설정과 원본 코드 조합별로 생성된 가상화 코드들을 포함하며, 각 샘플에 대해 정답 매핑(원본 코드 및 수행되는 동작)이 명확히 레이블링되어 있다。

그런 다음 이 데이터셋을 사용해 세 가지 대표적인 역난독화 기법의 정확도를 검증하였다:
(i) **VMHunt** (동적 패턴으로 VM 경계 식별),
(ii) **Generic Deobfuscator**~\cite{yadegari2015generic} (정적 변환 규칙 기반 역난독화),
(iii) **Syntia** (신경망/프로그램 합성을 이용한 역공학)。
평가 결과, **이들 기법 모두 최신 상용 난독화에 충분히 효과적이지 않음**이 드러났다. 요약하면 다음과 같다。

VMHunt의 경우, Code Virtualizer와 Themida가 삽입한 **가짜 컨텍스트 스위치**에 현혹되어 실제 가상 코드의 경계를 놓치는 문제가 발견되었다. 즉, 난독화기가 의도적으로 만들어낸 미끼에 의해 VMHunt의 핵심 가정 (컨텍스트 스위치 한 쌍이 VM 범위의 표시)을 악용당한 것이다. Generic Deobfuscator의 경우 추출된 가상 명령을 원본으로 치환하는 규칙 매칭이 정밀하지 못해, 많은 경우에 잘못된 변환을 하거나 실패하였다. Syntia의 경우, 짧은 가상 코드 조각의 의미를 유추하는 데에는 어느 정도 성능을 보였지만, **junk 코드가 섞인 현실 사례에서는 정확한 식별에 어려움**을 겪었다. 특히 Syntia는 학습된 제한된 연산 조합 내에서만 추론하기 때문에, 난독화기가 변칙적으로 구현한 논리 조합 등에 대해 오판하거나, 일부 경우 원본과 다른 but 비슷한 기능을 가진 코드로 추측하여 오역하는 사례도 관찰되었다。전반적으로, 이들 기존 기법들은 체계적인 검증 없이 제안되어 실제 난독화된 프로그램에 적용했을 때 부정확하거나 누락이 발생함을 CIA를 통해 처음으로 명확히 지적한 것이다。

하지만 이러한 결과는 역으로, CIA로부터 얻은 지식을 활용하여 기존 기법들을 개선할 수 있는 가능성도 보여준다. 예를 들어, CIA가 찾아낸 가짜 컨텍스트 스위치 패턴을 VMHunt에 반영하면 오탐지를 줄일 수 있을 것이고, CIA의 매핑 규칙 DB를 Generic Deobfuscator에 통합하면 훨씬 광범위한 명령 치환을 정확히 수행할 수 있을 것이다. Syntia 역시 CIA의 데이터셋으로 학습/테스트함으로써 더 현실적인 대응이 가능해질 것으로 기대된다。요컨대 RQ3에 대한 답은, CIA가 추출한 지식이 기존 역난독화 기법들의 한계를 드러낼 뿐 아니라, 그들을 보완/향상시키는 데 유용하게 쓰일 수 있음을 의미한다。

## 결론 및 향후 연구 방향

본 논문에서는 상용 코드 가상화 난독화 도구의 내부 지식을 자동으로 학습하는 **Chosen-Instruction Attack (CIA)** 기법을 제안하였다. CIA는 공격자가 난독화기에 선택한 입력을 주어보는 능동적 접근을 통해 *앵커 명령어*로 가상화 코드 영역을 식별하고, *Guided Simplification*으로 핵심 명령을 추출함으로써, **대량의 재사용 가능한 지식**을 효과적으로 획득할 수 있음을 보였다. 실험을 통해 실제 상용 난독화기들의 복잡한 내부 구조와 매핑 규칙 수백 가지를 밝혀냈으며, 이 지식을 활용하여 난독화 분석 분야의 첫 벤치마크를 구축하고 기존 기법들의 한계를 체계적으로 평가하였다。그 결과, 최신 난독화 기법의 복잡성은 현재 공개된 역난독화 기술의 능력을 크게 상회하며, 난독화 해독을 위해서는 CIA와 같은 **체계적 지식 학습 접근법이 필수적**임을 확인하였다。

향후 연구로는 CIA의 **적용 범위 확대**와 **현실적 제한점 보완**이 논의된다. 첫째, CIA는 공격자가 난독화기에 임의 코드를 넣어볼 수 있어야 하는데, 만약 난독화기가 클라우드 서비스 형태로 제공되거나 폐쇄적 환경에 있다면 공격에 제약이 있을 수 있다。이러한 *비접근성 난독화기*에 대한 대안적 접근 방법, 혹은 제한적인 상호작용으로도 지식을 캐낼 수 있는 기법이 향후 과제로 남는다. 둘째, 본 연구는 x86 코드를 대상으로 했지만, **RISC 계열 아키텍처**의 경우 명령어 집합이 단순하여 앵커로 활용할 수 있는 후보가 적을 수 있다。ARM 혹은 안드로이드 환경의 난독화에 CIA를 적용하려면 해당 플랫폼에 맞는 앵커 발굴이나 기법 조정이 필요하며, 저자들도 이러한 방향으로의 CIA 확장을 미래 과제로 언급하고 있다. 마지막으로, 상용 난독화 도구 개발자들도 CIA에 대응하여 진화할 가능성이 있다. 예컨대, CIA에서 앵커로 사용하는 특수 명령어들까지 가상화하도록 업그레이드하거나, 공격자의 패턴 입력을 감지해 난독화 출력을 교란시키는 등의 대응책을 고려할 수 있다. 따라서 공격자 측에서도 이에 맞서는 연구(예: 앵커 다양화, 적대적 입력 생성 등)가 지속될 필요가 있다。

그럼에도 불구하고, CIA가 보여준 핵심 성과는 **“난독화 해독을 위한 체계적인 지식 획득”**이라는 새 패러다임을 제시했다는 점이다. 이는 향후 악성코드 분석, 소프트웨어 보호 연구, 역공학 교육 등에 많은 영향을 미칠 것으로 전망된다. 또한 CIA로 구축된 풍부한 데이터셋과 지식베이스는 차세대 역난독화 도구 개발에 귀중한 자산으로 활용될 수 있다. 결론적으로, Chosen-Instruction Attack은 VM 기반 난독화의 난해함에 맞서 *공격자가 주도적으로 지식을 학습하고 활용*하는 길을 개척했으며, 이는 소프트웨어 보안 분야에서 난독화에 대응하는 새로운 지평을 열었다고 하겠다。

## GitHub 구현 코드 분석 (CIA-toolchain)

논문의 기법은 *CIA-toolchain*이라는 이름으로 오픈 소스 구현이 제공되어 있다 (저자들은 NDSS 2022 논문과 함께 도구 코드를 공개했다)。 해당 GitHub 리포지토리([https://github.com/chosen-instruction-attack/CIA-toolchain](https://github.com/chosen-instruction-attack/CIA-toolchain))를 분석하여, CIA 기법이 실제로 어떻게 구현되었는지 살펴본다。

### 리포지토리 구조

CIA-toolchain 저장소의 최상위 디렉토리에는 다음과 같은 주요 폴더들이 존재한다：

- **`auto_gen/`**:
  CIA에서 **지식 유출 입력 프로그램들을 자동 생성**하기 위한 스크립트와 도구들이 들어있다。여기에는 다양한 원본 명령어 $c$를 조합하여 템플릿 코드에 삽입하고, 각 난독화 도구별로 컴파일/난독화 준비를 하는 코드가 포함된다。

- **`pintools/`**:
  **실행 trace 수집을 위한 동적 계측 도구**들이 들어있다. *Intel PIN* 프레임워크를 이용하여 작성된 C/C++ 기반의 Pintool들이며, 주로 $O(P_c)$ 실행 시 앵커를 감지하고 그 사이의 명령어들을 기록하는 기능을 구현한다。 (예: `instracelog.cpp`라는 Pintool은 명령 실행을 훅하여 trace를 파일로 출력하는 역할을 한다。)

- **`contrast/`**:
  수집된 trace를 입력 받아 **Guided Simplification과 검증을 수행하는 분석 코드**가 위치한다. Python 언어로 작성되어 있으며, `angr`와 `Z3` SMT 솔버를 사용하여 trace 내 명령어들을 분석/정리하고 원본 명령어와의 동치성을 검사하는 로직이 구현되어 있다. 이 폴더에는 Algorithm 1에 해당하는 핵심 알고리즘 및 부가적인 분석 유틸리티가 포함된다。

- **그 외**: `LICENSE` (BSD 2-Clause 라이선스 명시), `README.md` (간단 안내), `.gitignore` 등이 있다。README에는 구체적인 사용법보다는 “Chosen-instruction attack의 툴체인”이라는 소개만 간략히 있다。

또한 저자들은 난독화 결과와 추출한 지식으로 구성된 **CIA-dataset**도 별도 저장소로 공개하였으며, CIA-toolchain의 GitHub Pages ([https://chosen-instruction-attack.github.io](https://chosen-instruction-attack.github.io))에 툴체인 사용법과 문서를 제공하고 있다。 (예: 요구 환경, 앵커 명령어 목록, 난독화기별 설정법 등이 문서화되어 있다。)

### 실행 방법 및 예제 사용법

CIA-toolchain을 실행하기 위해서는 몇 가지 전제 조건이 필요하다. 먼저 공격 대상이 되는 상용 난독화 도구(VMProtect, Themida 등)를 사용 가능한 상태여야 하며, 해당 도구들의 가상화 매크로나 옵션을 활용할 수 있어야 한다. 또한 실행 trace 수집을 위해 Intel PIN (동적 바이너리 계측 도구)을 설치해야 하고, 가상화된 바이너리를 로드하여 추적할 수 있는 환경(대개 Windows PE 바이너리를 대상으로 하므로 Windows 또는 호환 환경)이 필요하다。마지막으로 Python 분석을 위한 `angr` 라이브러리와 SMT 솔버(Z3)가 설치되어 있어야 한다. 요약하면 **요구사항: 상용 난독화기 + PIN + angr (Python) + XED**이다。

일반적인 CIA-toolchain 사용 흐름은 다음과 같다：

1. **지식 유출 입력 프로그램 생성**:
   `auto_gen/`의 스크립트 또는 템플릿 코드를 이용하여, 목표로 하는 원본 명령어($c$)를 포함하는 입력 C/어셈블리 프로그램을 생성한다. 이때 해당 난독화기의 가상화 시작/종료 매크로로 $c$ 부분을 감싸고, 앵커 명령어를 앞뒤로 삽입한다。`auto_gen` 모듈은 Intel의 **XED (X86 Encoder-Decoder)** 라이브러리를 이용하여 유효한 어셈블리 코드를 생성하거나, 명령어 목록을 순회하면서 다양한 $c$를 만들어내는 기능을 제공한다。 (예: Python 스크립트를 실행하면 미리 정의된 명령 리스트를 템플릿에 삽입하여 여러 소스 코드 파일 생성)

2. **입력 프로그램 컴파일 및 난독화**:
   생성된 입력 프로그램을 해당 난독화기에서 요구하는 방식으로 컴파일한다 (대부분의 상용 난독화기는 PE 실행 파일을 인풋으로 받으므로, Visual C++ 등의 컴파일러로 EXE 생성)。그런 다음, 상용 난독화 도구를 사용하여 그 실행 파일을 보호한다。일부 도구는 GUI만 제공하지만 대체로 커맨드라인 인터페이스나 스크립팅 기능이 있어 자동화 가능하다。 (예: VMProtect는 명령줄에서 프로젝트 파일을 넘겨 실행할 수 있고, Themida는 스크립트 엔진이 있음) 이렇게 해서 **난독화된 실행 파일 $O(P_c)$**를 얻는다。

3. **실행 및 trace 수집**:
   Intel PIN을 이용하여 난독화된 프로그램 $O(P_c)$를 실행하면서, `pintools/`에 구현된 *InstraceLog* 같은 Pintool을 사용해 명령어 trace를 기록한다。Pintool은 실행되는 모든 명령어의 주소, 코드, 레지스터/메모리 값 등을 캡처할 수 있는데, CIA에서는 최적화로서 처음 등장하는 앵커 명령어를 만나기 전까지는 기록을 건너뛰고, 앵커를 만나면 기록을 시작하여 두 번째 앵커가 나오면 기록을 중지하는 논리를 구현한다。이렇게 하면 자동으로 **앵커 사이의 trace만** 파일로 남게 된다。 PIN을 이용한 trace 수집은 명령줄에서 다음과 같이 수행된다 (예시)：

   ```bash
   pin -t pintools/instracelog.dll -o trace.out -- O(P_c).exe
   ```

   위 명령은 `instracelog` Pintool을 사용하여 `O(P_c).exe`를 실행하고, 결과 trace를 `trace.out` 파일로 저장한다。

4. **trace 분석 및 지식 추출**:
   수집된 trace 파일 (`trace.out`)을 `contrast/`의 Python 스크립트에 넘겨준다。 예를 들어 `contrast/analyze.py trace.out config.json` 형태로 실행하면, 해당 스크립트가 trace를 파싱하고 Guided Simplification 알고리즘을 적용하여 핵심 명령어 시퀀스를 추출한다。`config.json` 등에는 원본 명령어 $c$의 정보나 난독화기 종류 등의 메타데이터가 포함되어 있을 수 있다。스크립트는 angr 라이브러리를 통해 trace 내 명령어를 기호 실행함으로써 backward slicing 등을 수행하고, 최종 추출된 가상 명령어 블록을 검증한 후 콘솔 출력이나 파일로 기록한다。이러한 결과에는 “원본 명령어 X는 난독화기 Y에서 명령 시퀀스 Z로 매핑됨”과 같은 정보가 담긴다。

5. **지식 축적 및 활용**:
   이러한 과정을 다양한 $c$에 대해 반복 수행함으로써, 매핑 규칙 DB가 쌓인다。 CIA-toolchain 자체는 이러한 DB를 파일로 축적하거나, 나아가 `CIA-dataset` 생성에 활용하도록 설계되었다。 사용자는 추출된 매핑들을 모아서 새로운 프로그램의 역난독화에 참고하거나, 기존 도구의 정확도를 검증하는 등 활용할 수 있다。

위 과정의 일부는 현재 반자동적으로 이루어질 수도 있다。 (예: 난독화 도구 실행을 수동으로 해야 할 수 있음, GUI 기반 도구는 사용자 개입 필요 등。) 그러나 전반적인 파이프라인은 **“입력 생성 $\rightarrow$ 난독화 $\rightarrow$ 실행 추적 $\rightarrow$ 분석”**의 흐름으로 구성되어 있으며, 각 단계를 담당하는 코드가 CIA-toolchain에 포함되어 있다。

### 논문 내용과 구현의 연관성

CIA-toolchain의 구성 요소들은 논문에서 제안된 CIA 기법의 단계들과 정확히 대응된다。 `auto_gen` 모듈은 논문에서의 **CIA 샘플 구성 단계** (Section IV-A)에 해당하며, Intel XED를 통해 다양한 유효 명령어를 생성하고 C 템플릿에 주입하는 기능은 논문에 언급된 “Intel XED를 활용한 유효 명령 생성”을 구현한 것이다。 생성된 코드에 앵커와 지식 유출 코드를 배치하고 난독화 매크로를 넣는 작업 역시 auto_gen에서 자동화되어 있는데, 이는 실험 대상 난독화기마다 약간씩 다른 인터페이스를 맞추기 위한 설정(예: VMProtect의 VMProtectBegin, Themida의 VMEnter 등)을 포함한다。이러한 설정 정보는 CIA-toolchain의 문서(`docs/obfuscator_configurations`)에 정리되어 있으며, 각 난독화 제품별 앵커 허용 여부, 매크로 사용법 등이 코드에 반영되어 있다。

`pintools` 폴더의 내용은 **동적 실행 및 앵커 기반 trace 수집**을 담당하며, 이는 논문의 Section IV-B에 해당한다。 특히 `instracelog.cpp`는 PIN Tool로 작성된 C++ 코드로, 프로그램의 매 명령어 수행 시 호출되어 명령어 주소, 바이트, 레지스터 상태 등을 기록한다。 이 코드에는 논문에서 언급된 “앵커 명령어 감지 후 trace 기록 시작/종료” 로직이 구현되어 있다。 예를 들어, `instracelog.cpp` 내의 주요 함수 중 하나는 `Instruction()`으로, PIN의 `INS_AddInstrumentFunction` API를 통해 각 명령어 실행 직전에 호출되며, 명령어가 앵커인지 확인하는 조건문과 trace 기록을 위한 파일 출력 로직이 포함되어 있다。 또한 PIN의 `TRACE_AddInstrumentFunction`을 사용해 기본 블록 단위로 분석하는 최적화 기법도 적용되어 있어, 불필요한 명령어 기록을 줄이고 효율성을 높였다。

`contrast` 폴더는 논문의 **Guided Simplification과 검증 단계** (Section IV-C)를 구현한다。 `analyze.py`는 trace 파일을 읽어들여 명령어 시퀀스를 파싱하고, `angr`를 이용해 기호 실행 기반으로 backward slicing을 수행한다。 여기서 `angr.Project` 객체를 생성하여 바이너리를 로드하고, `angr.SimulationManager`를 통해 명령어 시퀀스의 상태 변화를 추적한다。 이후 Z3 솔버를 호출하여 원본 명령어 $c$와 추출된 가상 명령어 시퀀스 $S$ 간의 동치성을 확인하는 로직이 구현되어 있다。 예를 들어, `contrast/validate.py` 내의 함수는 Z3의 `Solver` 객체를 생성하고, 두 시퀀스의 출력 상태를 비교하는 논리식을 구성한 후 `check()` 메소드로 동치 여부를 판단한다。 이 과정에서 junk 코드 제거를 위해 slicing 및 정규화(normalization)가 적용되며, 논문의 Algorithm 1이 코드로 직접 변환된 형태를 볼 수 있다。

Möbius Strip Reverse Engineering
Street AddressCity, State, ZipPhone Number
Training ClassesResearchBlogContactSign In
VMProtect, Part 0: Basics
August 6, 2008 Rolf Rolles
Originally published on August 6th, 2008 on OpenRCE

This is part #0 of a four-part series on VMProtect. The other parts can be found here:

Part 0: Basics
Part 1: Bytecode and IR
Part 2: Primer on Optimization
Part 3: Optimizing and Compiling
VMProtect is a virtualization protector.  Like other protections in the genre, among others ReWolf's x86 Virtualizer and CodeVirtualizer, it works by disassembling the x86 bytecode of the target executable and compiling it into a proprietary, polymorphic bytecode which is executed in a custom interpreter at run-time.  This is unlike the traditional notions of packing, in which the x86 bytecode is simply encrypted and/or compressed:  with virtualization, the original x86 bytecode in the protected areas is gone, never to be seen again.  Or so the idea goes.

If you've never looked at VMProtect before, I encourage you to take a five-minute look in IDA (here's a sample packed binary).  As far as VMs go, it is particularly skeletal and easily comprehended.  The difficulty lies in recreating working x86 bytecode from the VM bytecode.  Here's a two-minute analysis of its dispatcher.

push edi; push all registers
push ecx
push edx
push esi
push ebp
push ebx
push eax
push edx
pushf
push 0 ; imagebase fixup
mov esi, [esp+8+arg_0] ; esi = pointer to VM bytecode
mov ebp, esp ; ebp = VM's "stack" pointer
sub esp, 0C0h
mov edi, esp ; edi = "scratch" data area

VM__FOLLOW__Update:
add esi, [ebp+0]

VM__FOLLOW__Regular:
mov al, [esi]; read a byte from EIP
movzx eax, al
sub esi, -1; increment EIP
jmp ds:VM__HandlerTable[eax*4] ; execute instruction handler
A feature worth discussing is the "scratch space", referenced by the register edi throughout the dispatch loop.  This is a 16-dword-sized area on the stack where VMProtect saves the registers upon entering the VM, modifies them throughout the course of a basic block, and from whence it restores the registers upon exit.  For each basic block protected by the VM, the layout of the registers in the scratch space can potentially be different.

Here's a disassembly of some instruction handlers.  Notice that A) VMProtect is a stack machine and that B) each handler -- though consisting of scant few instructions -- performs several tasks, e.g. popping several values, performing multiple operations, pushing one or more values.

```text
#00:x = [EIP-1] & 0x3C; y = popd; [edi+x] = y

.text:00427251 and al, 3Ch; al = instruction number
.text:00427254 mov edx, [ebp+0] ; grab a dword off the stack
.text:00427257 add ebp, 4 ; pop the stack
.text:0042725A mov [edi+eax], edx ; store the dword in the scratch space

#01:x = [EIP-1] & 0x3C; y = [edi+x]; pushd y

.vmp0:0046B0EB and al, 3Ch; al = instruction number
.vmp0:0046B0EE mov edx, [edi+eax] ; grab a dword out of the scratch space
.vmp0:0046B0F1 sub ebp, 4 ; subtract 4 from the stack pointer
.vmp0:0046B0F4 mov [ebp+0], edx ; push the dword onto the stack

#02:x = popw, y = popw, z = x + y, pushw z, pushf

.text:004271FB mov ax, [ebp+0] ; pop a word off the stack
.text:004271FF sub ebp, 2
.text:00427202 add [ebp+4], ax ; add it to another word on the stack
.text:00427206 pushf
.text:00427207 pop dword ptr [ebp+0] ; push the flags

#03:x = [EIP++]; w = popw; [edi+x] = Byte(w)

.vmp0:0046B02A movzx eax, byte ptr [esi] ; read a byte from EIP
.vmp0:0046B02D mov dx, [ebp+0] ; pop a word off the stack
.vmp0:0046B031 inc esi ; EIP++
.vmp0:0046B032 add ebp, 2; adjust stack pointer
.vmp0:0046B035 mov [edi+eax], dl ; write a byte into the scratch area

#04:x = popd, y = popw, z = x << y, pushd z, pushf

.vmp0:0046B095 mov eax, [ebp+0]; pop a dword off the stack
.vmp0:0046B098 mov cl, [ebp+4] ; pop a word off the stack
.vmp0:0046B09B sub ebp, 2
.vmp0:0046B09E shr eax, cl ; shr the dword by the word
.vmp0:0046B0A0 mov [ebp+4], eax; push the result
.vmp0:0046B0A3 pushf
.vmp0:0046B0A4 pop dword ptr [ebp+0] ; push the flags

#05:x = popd, pushd ss:[x]

.vmp0:0046B5F7 mov eax, [ebp+0]; pop a dword off the stack
.vmp0:0046B5FA mov eax, ss:[eax] ; read a dword from ss
.vmp0:0046B5FD mov [ebp+0], eax; push that dword
```

← VMProtect, Part 1: Bytecode and IRCompiler 1, X86 Virtualizer 0 →
Comments (2)Newest First
Newest First
Preview Post Comment…
Sneeker 5 months ago · 0 Likes

Thanks for this free material, it's great to follow along!

Pizza Recipes 11 months ago · 0 Likes

Great bloog you have

Powered by Squarespace

Möbius Strip Reverse Engineering
Street AddressCity, State, ZipPhone Number
Training ClassesResearchBlogContactSign In
VMProtect, Part 1: Bytecode and IR
August 6, 2008 Rolf Rolles
Originally posted on August 6th, 2008 on OpenRCE

This is part #1 of a four-part series on VMProtect. The other parts can be found here:

Part 0: Basics
Part 1: Bytecode and IR
Part 2: Primer on Optimization
Part 3: Optimizing and Compiling
The approach I took with ReWolf's x86 Virtualizer is also applicable here, although a more sophisticated compiler is required.  What follows is some preliminary notes on the design and implementation of such a component.  These are not complete details on breaking the protection; I confess to having only looked at a few samples, and I am not sure which protection options were enabled.

As before, we begin by constructing a disassembler for the interpreter.  This is immediately problematic, since the bytecode language is polymorphic.  I have created an IDA plugin that automatically constructs OCaml source code for a bytecode disassembler.  In a production-quality implementation, this should be implemented as a standalone component that returns a closure.

The generated disassembler, then, looks like this:

let disassemble bytearray index =
match (bytearray.(index) land 0xff) with
| 0x0-> (VM__Handler0__PopIntoRegister(0),[index+1])
| 0x1-> (VM__Handler1__PushDwordFromRegister(0),[index+1])
| 0x2-> (VM__Handler2__AddWords,[index+1])
| 0x3-> (VM__Handler3__StoreByteIntoRegister(bytearray.(index+1)),[index+2])
| 0x4-> (VM__Handler0__PopIntoRegister(4),[index+1])
| 0x5-> (VM__Handler1__PushDwordFromRegister(4),[index+1])
| 0x6-> (VM__Handler4__ShrDword,[index+1])
| 0x7-> (VM__Handler5__ReadDword__FromStackSegment,[index+1])
| ...-> ...
Were we to work with the instructions individually in their natural granularity, depicted above, the bookkeeping on the semantics of each would likely prove tedious.  For illustration, compare and contrast handlers #02 and #04.  Both have the same basic pattern:  pop two values (words vs. dwords), perform a binary operation (add vs. shr), push the result, then push the flags.  The current representation of instructions does not express these, or any, similarities.

Handler #02:           Handler #04:
mov ax, [ebp+0]        mov eax, [ebp+0]
sub ebp, 2             mov cl, [ebp+4]
add [ebp+4], ax        sub ebp, 2
pushf                  shr eax, cl
pop dword ptr [ebp+0]  mov [ebp+4], eax
pushf
pop dword ptr [ebp+0]
Therefore, we pull a standard compiler-writer's trick and translate the VMProtect instructions into a simpler, "intermediate" language (hereinafter "IR") which resembles the pseudocode snippets atop the handlers in part zero.  Below is a fragment of that language's abstract syntax.

```ocaml
type size = B | W | D | Q
type temp = int * size
type seg= Scratch | SS | FS | Regular
type irbinop= Add | And | Shl | Shr | MakeQword
type irunop= Neg | MakeByte | TakeHighDword | Flags
type irexpr =
| Reg of register
| Temp of int
| Const of const
| Deref of seg * irexpr * size
| Binop of irexpr * irbinop * irexpr
| Unop of irexpr * irunop

type ir =
DeclareTemps of temp list
| Assign of irexpr * irexpr
| Push of irexpr
| Pop of irexpr
| Return
```

A portion of the VMProtect -> IR translator follows; compare the translation for handlers #02 and #04.

```ocaml
let make_microcode = function
VM__Handler0__PopIntoRegister(b) -> [Pop(Deref(Scratch, Const(Dword(zero_extend_byte_dword(b land 0x3C))), D))]
| VM__Handler2__AddWords -> [DeclareTemps([(0, W);(1, W);(2, W)]);
 Pop(Temp(0));
 Pop(Temp(1));
 Assign(Temp(2), Binop(Temp(0), Add, Temp(1)));
 Push(Temp(2));
 Push(Unop(Temp(2), Flags))]
| VM__Handler4__ShrDword -> [DeclareTemps([(0, D);(1, W);(2, D)]);
 Pop(Temp(0));
 Pop(Temp(1));
 Assign(Temp(2), Binop(Temp(0), Shr, Temp(1)));
 Push(Temp(2));
 Push(Unop(Temp(2), Flags))]
| VM__Handler7__PushESP-> [Push(Reg(Esp))]
| VM__Handler23__WriteDwordIntoFSSegment -> [DeclareTemps([(0, D);(1, D)]);
 Pop(Temp(0));
 Pop(Temp(1));
 Assign(Deref(FS, Temp(0), D), Temp(1))]
| (*...*) -> (*...*)
```

To summarize the process, below is a listing of VMProtect instructions, followed by the assembly code that is executed for each, and to the right is the IR translation.

VM__Handler1__PushDwordFromRegister 32

; Push (Deref (Scratch, Const (Dword 32l), D));

and al, 3Ch ; al = 32
mov edx, [edi+eax]
sub ebp, 4
mov [ebp+0], edx

VM__Handler7__PushESP
; Push (Reg Esp);
mov eax, ebp
sub ebp, 4
mov [ebp+0], eax

VM__Handler0__PopIntoRegister 40
; Pop (Deref (Scratch, Const (Dword 40l), D));
and al, 3Ch
mov edx, [ebp+0]
add ebp, 4
mov [edi+eax], edx

VM__Handler19__PushSignedByteAsDword (-1l)
; Push (Const (Dword (-1l)));
movzx eax, byte ptr [esi] ; *esi = -1
sub esi, 0FFFFFFFFh
cbw
cwde
sub ebp, 4
mov [ebp+0], eax

VM__Handler9__PushDword 4525664l
; Push (Const (Dword 4525664l));
mov eax, [esi] ; *esi = 4525664l
add esi, 4
sub ebp, 4
mov [ebp+0], eax

VM__Handler9__PushDword 4362952l};
; Push (Const (Dword 4362952l));
mov eax, [esi] ; *esi = 4362952l
add esi, 4
sub ebp, 4
mov [ebp+0], eax

VM__Handler19__PushSignedByteAsDword 0l};
; Push (Const (Dword (0l)));
movzx eax, byte ptr [esi] ; *esi = 0
sub esi, 0FFFFFFFFh
cbw
cwde
sub ebp, 4
mov [ebp+0], eax

VM__Handler42__ReadDwordFromFSSegment};
;Pop (Temp 0);
;Push (Deref (FS, Temp 0, D));
mov eax, [ebp+0]DeclareTemps([(0,D)])
mov eax, fs:[eax]
mov [ebp+0], eax

← VMProtect, Part 2: Primer on OptimizationVMProtect, Part 0: Basics →
Comments (1)Newest First
Newest First
Preview Post Comment…
tFoster 4 years ago · 0 Likes

" I have created an IDA plugin that automatically constructs OCaml source code for a bytecode disassembler. In a production-quality implementation, this should be implemented as a standalone component that returns a closure."

Could you please provide a link?
Thanks in advance!

Powered by Squarespace

Möbius Strip Reverse Engineering
Street AddressCity, State, ZipPhone Number
Training ClassesResearchBlogContactSign In
VMProtect, Part 2: Primer on Optimization
August 6, 2008 Rolf Rolles
Originally published on OpenRCE on August 6th, 2008

This is part #2 of a four-part series on VMProtect. The other parts can be found here:

Part 0: Basics
Part 1: Bytecode and IR
Part 2: Primer on Optimization
Part 3: Optimizing and Compiling
Basically, VMProtect bytecode and the IR differ from x86 assembly language in four ways:

It's a stack machine;
The IR contains "temporary variables";
It contains what I've called a "scratch" area, upon which computations are performed rather than in the registers;
 In the case of VMProtect Ultra, it's obfuscated (or rather, "de-optimized") in certain ways.
It turns out that removing these four aspects from the IR is sufficient preparation for compilation into sensible x86 code.  We accomplish this via standard compiler optimizations applied locally to each basic block.  In general, there are a few main compiler optimizations used in this process.  The first one is "constant propagation".  Consider the following C code.

int x = 1;
function(x);
Clearly x will always be 1 when function is invoked:  it is defined on the first line, and is not re-defined before it is used in the following line (the definition in line one "reaches" the use in line two; alternatively, the path between the two is "definition-clear" with respect to x).  Thus, the code can be safely transformed into "function(1)".  If the first line is the only definition of the variable x, then we can replace all uses of x with the integer 1.  If the second line is the only use of the variable x, then we can eliminate the variable.

The next is "constant folding".  Consider the following C code.

int x = 1024;
function(x*1024);
By the above remarks, we know we can transform the second line into "function(1024*1024);".  It would be silly to generate code that actually performed this multiplication at run-time:  the value is known at compile-time, and should be computed then.  We can replace the second line with "function(1048576);", and in general we can replace any binary operation performed upon constant values with the computed result of that operation.

Similar to constant propagation is "copy propagation", as in the below.

void f(int i)
{
  int j = i;
  g(j);
}
The variable j is merely a copy of the variable i, and so the variable i can be substituted in for j until the point where either is redefined.  Lacking redefinitions, j can be eliminated entirely.

The next optimization is "dead-store elimination".  Consider the following C code.

int y = 2;
y = 1;
The definition of the variable y on line one is immediately un-done by the one on line two.  Therefore, there is no reason to actually assign the value 2 to the variable; the first store to y is a "dead store", and can be eliminated by the standard liveness-based compiler optimization known as "dead-store elimination", or more generally "dead-code elimination".

Here's an example from the VMProtect IR.

ecx = DWORD Scratch:[Dword(44)]
ecx = DWORD Scratch:[Dword(20)]
After dead-store-eliminating the first instruction, it turns out no other instructions use Scratch:[Dword(44)], and so its previous definition can be eliminated as well.

← VMProtect, Part 3: Optimization and Code GenerationVMProtect, Part 1: Bytecode and IR →
Comments (1)Newest First
Newest First
Preview Post Comment…
Vacationing Vicky 4 years ago · 0 Likes

Lovedd reading this thank you

Powered by Squarespace

Möbius Strip Reverse Engineering
Street AddressCity, State, ZipPhone Number
Training ClassesResearchBlogContactSign In
VMProtect, Part 3: Optimization and Code Generation
August 6, 2008 Rolf Rolles
Originally published on August 6th, 2008 on OpenRCE

This is part #3 of a four-part series on VMProtect. The other parts can be found here:

Part 0: Basics
Part 1: Bytecode and IR
Part 2: Primer on Optimization
Part 3: Optimizing and Compiling
The reader should inspect this unoptimized IR listing before continuing.  In an attempt to keep this entry from becoming unnecessarily long, the example snippets will be small, but for completeness a more thorough running example is linked throughout the text.

We begin by removing the stack machine features of the IR.  Since VMProtect operates on disassembled x86 code, and x86 itself is not a stack machine, this aspect of the protection is unnatural and easily removed.  Here is a 15-line fragment of VMProtect IR.

push Dword(-88)
push esp
push Dword(4)
pop t3
pop t4
t5 = t3 + t4
push t5
push flags t5
pop DWORD Scratch:[Dword(52)]
pop t6
pop t7
t8 = t6 + t7
push t8
push flags t8
pop DWORD Scratch:[Dword(12)]
pop esp
All but two instructions are pushes or pops, and the pushes can be easily matched up with the pops.  Tracking the stack pointer, we see that, for example, t3 = Dword(4).  A simple analysis allows us to "optimize away" the push/pop pairs into assignment statements.  Simply iterate through each instruction in a basic block and keep a stack describing the source of each push.  For every pop, ensure that the sizes match and record the location of the corresponding push.  We wish to replace the pop with an assignment to the popped expression from the pushed expression, as in

t3 = Dword(4)
t4 = esp
t7 = Dword(-88)
With the stack aspects removed, we are left with a more conventional listing containing many assignment statements.  This optimization substantially reduces the number of instructions in a given basic block (~40% for the linked example) and opens the door for other optimizations.  The newly optimized code is eight lines, roughly half of the original:

t3 = Dword(4)
t4 = esp
t5 = t3 + t4
DWORD Scratch:[Dword(52)] = flags t5
t6 = t5
t7 = Dword(-88)
t8 = t6 + t7
DWORD Scratch:[Dword(12)] = flags t8
esp = t8
A complete listing of the unoptimized IR versus the one with the stack machine features removed is here, which should be perused before proceeding.

Now we turn our attention to the temporary variables and the scratch area.  Recall that the former were not part of the pre-protected x86 code, nor the VMProtect bytecode -- they were introduced in order to ease the IR translation.  The latter is part of the VMProtect bytecode, but was not part of the original pre-protected x86 code.  Since these are not part of the languages we are modelling, we shall eliminate them wholesale.  On a high level, we treat each temporary variable, each byte of the scratch space, and each register as being a variable defined within a basic block, and then eliminate the former two via the compiler optimizations previously discussed.

Looking again at the last snippet of IR, we can see several areas for improvement.  First, consider the variable t6.  It is clearly just a copy of t5, neither of which are redefined before the next use in the assignment to t8.  Copy propagation will replace variable t6 with t5 and eliminate the former.  More generally, t3, t4, and t7 contain either constants or values that are not modified between their uses.  Constant and copy propagation will substitute the assignments to these variables in for their uses and eliminate them.

The newly optimized code is a slender three lines compared to the original 15; we have removed 80% of the IR for the running example.

DWORD Scratch:[Dword(52)] = flags Dword(4) + esp
esp = Dword(4) + esp + Dword(-88)
DWORD Scratch:[Dword(12)] = flags Dword(4) + esp + Dword(-88)
The side-by-side comparison can be found here.

The IR now looks closer to x86, with the exception that the results of computations are being stored in the scratch area, not into registers.  As before, we apply dead-store elimination, copy and constant propagation to the scratch area, removing dependence upon it entirely in the process.  See here for a comparison with the last phase.

Here is a comparison of the final, optimized code against the original x86:

push ebp                          push ebp
ebp = esp                         mov ebp, esp
push Dword(-1)                    push 0FFFFFFFFh
push Dword(4525664)               push 450E60h
push Dword(4362952)               push offset sub_4292C8
eax = DWORD FS:[Dword(0)]         mov eax, large fs:0
push eax                          push eax
DWORD FS:[Dword(0)] = esp         mov large fs:0, esp
eflags = flags esp + Dword(-88)
esp = esp + Dword(-88)            add esp, 0FFFFFFA8h
push ebx                          push ebx
push esi                          push esi
push edi                          push edi
DWORD SS:[Dword(-24) + ebp] = esp mov [ebp-18h], esp
call DWORD [Dword(4590300)]       call dword ptr ds:unk_460ADC
vmreturn Dword(0) + Dword(4638392)
Code generation is an afterthought.

← Bagle.W Thorough IDBVMProtect, Part 2: Primer on Optimization →
Comments (0)Newest First
Newest First
Preview Post Comment…
Powered by Squarespace
