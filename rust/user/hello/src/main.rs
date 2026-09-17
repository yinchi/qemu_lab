#![no_std]
#![no_main]

userlib::entry!(run);

fn run() {
    userlib::write(1, b"hello from userspace\n");
}
