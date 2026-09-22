#include <libproc.h>
#include <sys/proc_info.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <stdlib.h>
#include <errno.h>

/* 1 = matching listener, 0 = none, -1 = unavailable, -2 = permission denied.
 * family 0 inspects both IPv4 and IPv6 in one FD walk. */
int apphost_pid_listens(int pid, unsigned short port, int family) {
    errno = 0;
    int size = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, NULL, 0);
    if (size <= 0) {
        if (errno == EPERM || errno == EACCES) return -2;
        return (errno == 0 || errno == ESRCH || errno == ENOENT) ? 0 : -1;
    }
    if (size > 65536 * (int)sizeof(struct proc_fdinfo)) return -1;
    size += 32 * sizeof(struct proc_fdinfo);
    struct proc_fdinfo *fds = malloc(size);
    if (!fds) return -1;
    errno = 0;
    int used = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, fds, size);
    int result = 0;
    if (used <= 0) {
        if (errno == EPERM || errno == EACCES) result = -2;
        else if (errno != 0 && errno != ESRCH && errno != ENOENT) result = -1;
    }
    else if (used >= size || used % sizeof(*fds)) result = -1;
    else for (unsigned int i = 0; i < used / sizeof(*fds); i++) {
        if (fds[i].proc_fdtype != PROX_FDTYPE_SOCKET) continue;
        struct socket_fdinfo info;
        int n = proc_pidfdinfo(pid, fds[i].proc_fd, PROC_PIDFDSOCKETINFO, &info, sizeof(info));
        if (n != sizeof(info)) {
            if (errno == EPERM || errno == EACCES) result = -2;
            else if (errno != EBADF && errno != ENOENT && errno != ESRCH && result != -2) result = -1;
            continue;
        }
        int actual_family = info.psi.soi_family;
        if (info.psi.soi_kind != SOCKINFO_TCP ||
            (family && actual_family != family) ||
            (actual_family != AF_INET && actual_family != AF_INET6)) continue;
        struct tcp_sockinfo *tcp = &info.psi.soi_proto.pri_tcp;
        if (tcp->tcpsi_state != TSI_S_LISTEN || ntohs(tcp->tcpsi_ini.insi_lport) != port) continue;
        if (actual_family == AF_INET) {
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
