#set page(
  paper: "us-letter",
  margin: (top: 2.5cm, bottom: 2.5cm, left: 2.5cm, right: 2.5cm),
  header: context {
    if counter(page).get().first() > 0 {
      align(right, text(8.5pt, fill: rgb("#64748b"))[
        *Ethical and Legal Declarations* | Hanyang University
      ])
    }
  },
  footer: context {
    align(center, text(9pt, fill: rgb("#64748b"))[
      Page #counter(page).display("1 of 1")
    ])
  }
)

#set text(
  font: "Times New Roman",
  size: 10.5pt,
  lang: "en"
)

#set par(
  justify: true,
  leading: 0.65em
)

// Header
#align(center)[
  #text(15pt, weight: "bold")[ETHICAL AND LEGAL DECLARATIONS] \
  #v(2pt)
  #text(10pt, style: "italic", fill: rgb("#334155"))[Official Manuscript Submission Addendum]
  #v(0.4cm)
  #line(length: 100%, stroke: 0.8pt + rgb("#cbd5e1"))
]

#v(0.2cm)

#grid(
  columns: (auto, 1fr),
  row-gutter: 0.3cm,
  [*Manuscript Title:*], [ AETHER-64: Trace-Free Semantics-Preserving Devirtualization and Algebraic Deobfuscation for 64-Bit Protected Binaries],
  [*Author:*], [ Jeffrey Kim (Jeong Hoon Kim)],
  [*Affiliation:*], [ Department of Computer Science and Engineering, Hanyang University, Seoul, Republic of Korea],
  [*Corresponding Email:*], [ #text(fill: rgb("#2563eb"))[`jk9576@hanyang.ac.kr`]],
  [*Funding / Sponsorship:*], [ Independent Research (No External Grant, Committee, or Group Funding)],
)

#v(0.3cm)
#line(length: 100%, stroke: 0.5pt + rgb("#e2e8f0"))
#v(0.3cm)

*1. Compliance with Ethical Standards and Human/Animal Rights*
The author certifies that this research does not involve human subjects, vertebrate animals, clinical trials, or personally identifiable data (PII). In accordance with international academic publishing standards, formal Institutional Review Board (IRB) or animal ethics committee approval was not required.

*2. Dual-Use Technology and Responsible Research Statement*
The deobfuscation algorithms, symbolic analysis techniques, and software architectures described in this manuscript were developed and evaluated strictly for defensive cybersecurity purposes, automated malware forensics, academic verification, and binary transparency. The author adheres to responsible security disclosure practices. The research does not introduce exploits, attack payloads, or weaponized techniques intended to bypass authorization or compromise operational computer systems.

*3. Legal Compliance and Software Licensing*
All reverse engineering and binary analysis were performed within isolated, offline laboratory environments. Empirical evaluations utilized public open-source datasets (notably the Quarkslab Diffing Obfuscation Dataset and the Tigress Obfuscator suite) as well as legitimately licensed commercial binaries. No software was cracked, distributed, or modified in breach of applicable domestic or international intellectual property laws, the Digital Millennium Copyright Act (DMCA), or commercial End User License Agreements (EULAs). The methodologies utilized constitute non-infringing academic reverse engineering for interoperability and security verification.

*4. Conflict of Interest / Competing Interests Statement*
The author declares that there are no financial, personal, or professional affiliations, commitments, or competing interests that could have improperly influenced or biased the objectivity, analysis, or conclusions presented in this work.

*5. Data and Software Reproducibility*
The experimental benchmark suites, automated verification pipelines, and empirical evaluation data supporting the findings of this study have been made publicly available under open-source terms in the permanent repository at #link("https://github.com/omakasekim/VMP_DEVIRT")[https://github.com/omakasekim/VMP\_DEVIRT].

#v(0.8cm)

#align(right)[
  #block(width: 60%)[
    #line(length: 100%, stroke: 0.5pt + rgb("#94a3b8"))
    #v(0.1cm)
    *Jeffrey Kim (Jeong Hoon Kim)* \
    #text(9pt)[Department of Computer Science and Engineering] \
    #text(9pt)[Hanyang University, Seoul, Republic of Korea] \
    #text(9pt)[Date: October 24, 2026]
  ]
]
