# Morpheus - Bootloader

## Morpheus is a custom Bootloader project written in pure rust no_std.

* Why? you may ask. Understandable honestly i dont f*cking know why i do this to myself...

# The Vision:

This all started with a few dumb shellscripts i wrote, originally i wanted to create some shell utilitys,
which would enable me to distro hop or basically to hotswap distros. As you might guess: Thats a really dumb idea.

Nevertheless i didnt wanna give up on that thought so at first i considered playing with grub and doing some fancy grub init scripts or whatever, but i soon realized first of all grub wouldnt allow me todo what i want i want hotswapable distros via a command in your shell, no emulation no virtualization. I want a real bare metal kernel with persistant userland (/home) partition. This is how this repo came to live.

# Repo-Manifest 

And now im already 16k lines deep into the project and starting to question my sanity. Nevertheless a few of my biggest milestones are already behind me. As of v1.0.1 we actually can boot linux not just busybox but a full arch bootstrap image. le






