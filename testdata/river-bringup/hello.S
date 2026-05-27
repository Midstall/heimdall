.section .text
.global _start
_start:
    li a0, 0x42
    ebreak
    j _start
