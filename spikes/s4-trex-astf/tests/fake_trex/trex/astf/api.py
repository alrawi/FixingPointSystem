"""Recording stand-in for the TRex v3.08 ASTF profile API.

It mirrors the constructor and method signatures that the translator uses,
validates labels the way ASTFProgram does, and records every command, so tests
can execute generated profiles without a TRex installation.
"""


class ASTFError(Exception):
    pass


class ASTFProgram:
    def __init__(self, file=None, side="c", commands=None, stream=True, s_delay=None, udp_mtu=None, addon=None):
        self.side, self.stream = side, stream
        self.commands, self.labels, self.vars = [], {}, set()
        self.total_rcv_bytes = 0

    def _add(self, *cmd):
        self.commands.append(cmd)

    def connect(self):
        self._add("connect")

    def accept(self):
        self._add("connect")

    def send(self, buf, size=0, fill=None):
        assert isinstance(buf, (bytes, str)) and self.stream
        if size and size > len(buf):
            assert fill, "size without fill"
        self._add("send", max(len(buf), size))

    def send_msg(self, buf, size=0, fill=None):
        assert not self.stream
        self._add("send_msg", max(len(buf), size))

    def recv(self, bytes, clear=False):
        assert self.stream and isinstance(bytes, int) and bytes > 0
        self.total_rcv_bytes += bytes
        self._add("recv", self.total_rcv_bytes, clear)
        if clear:
            self.total_rcv_bytes = 0

    def recv_msg(self, pkts, clear=False):
        assert not self.stream
        self._add("recv_msg", pkts, clear)

    def delay(self, usec):
        self._add("delay", usec)

    def delay_rand(self, min_usec, max_usec):
        assert min_usec <= max_usec
        self._add("delay_rand", min_usec, max_usec)

    def reset(self):
        self._add("reset")

    def wait_for_peer_close(self):
        self._add("noclose")

    def set_var(self, var_id, val):
        assert isinstance(var_id, str) and isinstance(val, int)
        self.vars.add(var_id)
        self._add("set_var", var_id, val)

    def set_label(self, label):
        if label in self.labels:
            raise ASTFError(f"label {label} was defined already")
        self.labels[label] = len(self.commands)

    def jmp_nz(self, var_id, label):
        if label not in self.labels:
            raise ASTFError(f"label {label} wasn't defined")
        assert var_id in self.vars
        self._add("jmp_nz", var_id, self.labels[label])


class ASTFIPGenDist:
    def __init__(self, ip_range, distribution="seq", per_core_distribution=None):
        self.ip_range, self.distribution = ip_range, distribution


class ASTFIPGenGlobal:
    def __init__(self, ip_offset="1.0.0.0", ip_offset_server=None):
        self.ip_offset = ip_offset


class ASTFIPGen:
    def __init__(self, dist_client, dist_server, glob=None):
        self.dist_client, self.dist_server, self.glob = dist_client, dist_server, glob


class ASTFAssociationRule:
    def __init__(self, port=80, ip_start=None, ip_end=None, l7_map=None, assoc_id=None):
        self.port, self.ip_start, self.ip_end = port, ip_start, ip_end


class ASTFTCPClientTemplate:
    def __init__(self, ip_gen, cluster=None, program=None, port=80, cps=1, glob_info=None, limit=None, cont=None, core_base=None):
        self.ip_gen, self.program, self.port, self.cps = ip_gen, program, port, cps


class ASTFTCPServerTemplate:
    def __init__(self, program=None, assoc=None, glob_info=None):
        self.program, self.assoc = program, assoc


class ASTFTemplate:
    def __init__(self, client_template=None, server_template=None, tg_name=None):
        assert tg_name is None or len(tg_name) <= 20
        self.client_template, self.server_template, self.tg_name = client_template, server_template, tg_name


class ASTFProfile:
    def __init__(self, default_ip_gen, default_c_glob_info=None, default_s_glob_info=None, templates=None, cap_list=None, s_delay=None, udp_mtu=None):
        self.default_ip_gen, self.templates = default_ip_gen, templates or []
