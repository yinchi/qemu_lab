#ifndef REGISTERS_H
#define REGISTERS_H

/* PL011 UART0 base address on the QEMU "virt" machine */
#define UART0_BASE 0x09000000UL

/** Data register; writing transmits a character, reading receives one */
#define UART0_DR   (*(volatile unsigned int *)(UART0_BASE + 0x00))

/** Flag register; a read-only status register */
#define UART0_FR   (*(volatile unsigned int *)(UART0_BASE + 0x18))

/* Interrupt Mask Set/Clear register; writing 1 to a bit here enables the interrupt. */
#define UART0_IMSC (*(volatile unsigned int *)(UART0_BASE + 0x38))

/* Interrupt Clear register; writing 1 to a bit here clears the interrupt. */
#define UART0_ICR  (*(volatile unsigned int *)(UART0_BASE + 0x44))

#define UART0_FR_TXFF (1 << 5) /* Status mask: transmit buffer full? */
#define UART0_FR_RXFE (1 << 4) /* Status mask: receive buffer empty? */
#define UART0_RX    (1 << 4)   /* Interrupt mask: receive */

/* Generic Interrupt Controller v2 distributor, confirmed at 0x08000000 via device tree dump */
#define GICD_BASE 0x08000000UL
/* GICv2 distributor control register - master control for the entire distributor */
#define GICD_CTLR          (*(volatile unsigned int *)(GICD_BASE + 0x000))
/* GICv2 distributor (i)nterrupt (s)et-(enable) (r)egisters - each `int` sets 32 interrupts;
e.g. to enable interrupt 33, use `GICD_ISENABLER(1) = (1 << 1);`. Writing 0 bits
does not disable any interrupts. */
#define GICD_ISENABLER(n)  (*(volatile unsigned int *)(GICD_BASE + 0x100 + 4 * (n)))
/* GICv2 distributor interrupt priority registers - 1 byte (`char`) per interrupt from base,
priority value are thus 0-255 */
#define GICD_IPRIORITYR(n) (*(volatile unsigned char *)(GICD_BASE + 0x400 + (n)))
/* GICv2 distributor interrupt processor targets registers - 1 byte (`char`) per interrupt
from base; each `char` specifies the target CPU(s), 0-7 as bit flags */
#define GICD_ITARGETSR(n)  (*(volatile unsigned char *)(GICD_BASE + 0x800 + (n)))

/* GICv2 CPU interface, confirmed at 0x08010000 via device tree dump */
#define GICC_BASE 0x08010000UL
/* GICv2 CPU interface control register */
#define GICC_CTLR (*(volatile unsigned int *)(GICC_BASE + 0x000))
/* Interrupt Priority Mask Register - only interrupts with higher priority than this value are
signaled */
#define GICC_PMR  (*(volatile unsigned int *)(GICC_BASE + 0x004))
/* Interrupt Acknowledge Register - holds the interrupt ID of the highest priority pending
interrupt */
#define GICC_IAR  (*(volatile unsigned int *)(GICC_BASE + 0x00C))
/* End of Interrupt Register - writing the interrupt ID to it clears the interrupt */
#define GICC_EOIR (*(volatile unsigned int *)(GICC_BASE + 0x010))

#endif /* REGISTERS_H */