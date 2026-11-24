package core

import "golang.org/x/sys/unix"

func getTermios(fd int) (*unix.Termios, error) {
	return unix.IoctlGetTermios(fd, unix.TIOCGETA)
}

func setTermios(fd int, attributes *unix.Termios) error {
	return unix.IoctlSetTermios(fd, unix.TIOCSETA, attributes)
}

// cfmakeraw is the BSD libc cfmakeraw flag set that Apple's libc carries: unlike glibc it sets
// IGNBRK and also clears IMAXBEL and ECHOE.
func cfmakeraw(t *unix.Termios) {
	t.Iflag &^= unix.IMAXBEL | unix.IXOFF | unix.INPCK | unix.BRKINT | unix.PARMRK | unix.ISTRIP |
		unix.INLCR | unix.IGNCR | unix.ICRNL | unix.IXON | unix.IGNPAR
	t.Iflag |= unix.IGNBRK
	t.Oflag &^= unix.OPOST
	t.Lflag &^= unix.ECHO | unix.ECHOE | unix.ECHOK | unix.ECHONL | unix.ICANON | unix.ISIG |
		unix.IEXTEN | unix.NOFLSH | unix.TOSTOP | unix.PENDIN
	t.Cflag &^= unix.CSIZE | unix.PARENB
	t.Cflag |= unix.CS8 | unix.CREAD
	t.Cc[unix.VMIN] = 1
	t.Cc[unix.VTIME] = 0
}
