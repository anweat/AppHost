#include <libproc.h>
#include <sys/proc_info.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <stdlib.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>

/* 1 = matching listener, 0 = none, -1 = inspection incomplete. */
int apphost_pid_listens(int pid, unsigned short port, int family) {
    int size = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, NULL, 0);
    if (size < 0 || size > 65536 * (int)sizeof(struct proc_fdinfo)) return -1;
    if (!size) return (errno == ESRCH) ? 0 : -1;
    size += 32 * sizeof(struct proc_fdinfo);
    struct proc_fdinfo *fds = malloc(size);
    if (!fds) return -1;
    int used = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, fds, size);
    int result = 0;
    if (used <= 0 || used >= size || used % sizeof(*fds)) result = -1;
    else for (unsigned int i = 0; i < used / sizeof(*fds); i++) {
        if (fds[i].proc_fdtype != PROX_FDTYPE_SOCKET) continue;
        struct socket_fdinfo info;
        int n = proc_pidfdinfo(pid, fds[i].proc_fd, PROC_PIDFDSOCKETINFO, &info, sizeof(info));
        if (n != sizeof(info)) {
            if (errno != EBADF && errno != ENOENT && errno != ESRCH) result = -1;
            continue;
        }
        if (info.psi.soi_kind != SOCKINFO_TCP || info.psi.soi_family != family) continue;
        struct tcp_sockinfo *tcp = &info.psi.soi_proto.pri_tcp;
        if (tcp->tcpsi_state != TSI_S_LISTEN || ntohs(tcp->tcpsi_ini.insi_lport) != port) continue;
        if (family == AF_INET) {
            unsigned int addr = ntohl(tcp->tcpsi_ini.insi_laddr.ina_46.i46a_addr4.s_addr);
            if (addr != 0 && addr != INADDR_LOOPBACK) continue;
        } else {
            struct in6_addr *addr = &tcp->tcpsi_ini.insi_laddr.ina_6;
            if (!IN6_IS_ADDR_UNSPECIFIED(addr) && !IN6_IS_ADDR_LOOPBACK(addr)) continue;
        }
        result = 1; break;
    }
    free(fds);
    return result;
}

/* Probe wildcard addresses without SO_REUSEADDR: on macOS a reusable
 * loopback bind can succeed beside an existing wildcard listener. */
int apphost_port_available(unsigned short port, int family) {
    int fd = socket(family, SOCK_STREAM, 0);
    if (fd < 0) return errno;
    if (fcntl(fd, F_SETFD, FD_CLOEXEC) < 0) {
        int error = errno;
        close(fd);
        return error;
    }
    int rc;
    if (family == AF_INET) {
        struct sockaddr_in address = {0};
        address.sin_len = sizeof(address);
        address.sin_family = AF_INET;
        address.sin_port = htons(port);
        address.sin_addr.s_addr = htonl(INADDR_ANY);
        rc = bind(fd, (struct sockaddr *)&address, sizeof(address));
    } else {
        struct sockaddr_in6 address = {0};
        address.sin6_len = sizeof(address);
        address.sin6_family = AF_INET6;
        address.sin6_port = htons(port);
        address.sin6_addr = in6addr_any;
        rc = bind(fd, (struct sockaddr *)&address, sizeof(address));
    }
    int error = rc < 0 ? errno : 0;
    close(fd);
    return error;
}
