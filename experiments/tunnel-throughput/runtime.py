#!/usr/bin/env python3
"""Same NativeUDP/E2E/hop/path Runtime at controlled packet rate, synthetic I/O."""
import argparse
import json
import os
from pathlib import Path
import signal
import time

from run import (TunnelFixture, authenticated_graph, architecture, bounded_integer, cpu_details, cpu_window,
                 docker, docker_json, interrupted, now, performance_lock, platform,
                 wait_until, write_result, environment)

SUPERVISE = r"""
import sys, subprocess
sys.path.insert(0, '/run/meshway-lab')
import meshway_node as m
command = ['meshway-carrier-measure', 'runtime', '--config-file', '/run/meshway-lab/config.json',
           '--status-file', '/run/meshway-lab/status.json'] + sys.argv[1:]
child = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
identity = m.process_identity(child.pid)
if identity:
 m.save('agent-pid.json', {'pid':child.pid, 'start_ticks':identity['start_ticks']})
m.save('agent-exit.json', {'exit_code':child.wait()})
"""


class RuntimeFixture(TunnelFixture):
    def kernel(self):
        sample = "import json; from pathlib import Path; r=Path('/run/meshway-lab'); print(json.dumps({'pid':json.loads((r/'agent-pid.json').read_text())['pid'], 'files':{n:(Path('/proc/net')/n).read_text() for n in ['snmp','netstat','softnet_stat']}}))"
        return {n:{'proc':json.loads(self.exec(n,'python3','-c',sample).stdout),
                   'udp_memory':self.exec(n,'ss','-u','-a','-n','-m').stdout} for n in self.nodes}

    def start_agent(self, node, path=None, pps=20000):
        self.agents_started.add(node)
        args = [] if path is None else ['--path',path,'--seconds',str(self.args.seconds),'--packets-per-second',str(pps)]
        docker('exec','--detach','--env',f'TOKIO_WORKER_THREADS={self.args.workers}',self.nodes[node],
               'python3','-c',SUPERVISE,*args)

    def raw_status(self):
        return {n:json.loads(self.exec(n,'cat','/run/meshway-lab/status.json').stdout) for n in self.nodes}

    def setup_relay(self):
        for net in ['a','b']: self.create_network(net,'runtime-relay')
        self.create_node('NYC','a','runtime-relay')
        self.create_node('LAX','a','runtime-relay')
        docker('network','connect',self.networks['b']['name'],self.nodes['LAX'])
        self.create_node('SHA','b','runtime-relay')
        self.remove_defaults()
        self.generate_configs('udp')
        for n in ['LAX','NYC','SHA']: self.start_agent(n)
        wait_until('Runtime authenticated relay graph',lambda:authenticated_graph(self.statuses()))
        return {'statuses':self.statuses(),'connections':self.connections()}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image',required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--temp-dir',type=Path,required=True)
    p.add_argument('--seconds',type=bounded_integer(1,30),default=10)
    p.add_argument('--queue',type=bounded_integer(1,256),default=64)
    p.add_argument('--udp-receive-buffer',type=bounded_integer(0,4*1024*1024),default=0)
    args = p.parse_args()
    args.runtime_only,args.cpus,args.memory,args.mtu,args.workers = True,1.,'256m',1500,2
    args.work_budget,args.first_leg,args.native_tcp_nodelay,args.ready_timeout = 16,'udp','default',30
    os.umask(0o077)
    if args.output.exists(): raise ValueError('Output already exists')
    signal.signal(signal.SIGTERM,interrupted)
    info = docker_json('info','--format','{{json .}}')
    native = architecture(info['Architecture'])
    if native != architecture(platform.machine()): raise ValueError('native Docker required')
    result = {'run_id':'meshway-runtime-'+os.urandom(8).hex(),'status':'running','started_at':now(),
              'settings':{k:str(v) if isinstance(v,Path) else v for k,v in vars(args).items()},
              'scope':'NativeUDP same Runtime/hop Noise/E2E/ordered path; synthetic 1280-byte payloads, NOT TCP and NOT a TUN overhead retention ratio.',
              'environment':environment(info),'cases':[],'fixtures':{},'cleanup_errors':[]}
    f = None
    try:
        for mode in ['direct','relay']:
            f = RuntimeFixture(args,result)
            f.prepare_image(native)
            result['fixtures'][mode] = f.setup_direct() if mode=='direct' else f.setup_relay()
            for pps in [20000,40000,64000]:
                for reverse in [False,True]:
                    source,target = ('SHA','NYC') if reverse else ('NYC','SHA')
                    f.agent_helper(source,'stop')
                    wait_until('source stopped',lambda:not f.agent_state(source)['running'])
                    status_before = f.raw_status()
                    kernel_before = f.kernel()
                    cpu_before = f.cgroups()
                    path = ','.join([source.lower(),*(['lax'] if mode=='relay' else []),target.lower()])
                    f.start_agent(source,path,pps)
                    # Wait for this newly spawned generation, not its old PID.
                    old_pid = kernel_before[source]['proc']['pid']
                    wait_until('sender generation',lambda:f.agent_state(source).get('pid') != old_pid)
                    wait_until('sender finished',lambda:not f.agent_state(source)['running'],timeout=args.seconds+30)
                    sender_exit=f.agent_state(source).get('exit',{}).get('exit_code')
                    if sender_exit != 0: raise RuntimeError(f'Synthetic sender failed (exit {sender_exit}); private logs suppressed')
                    cpu_after = f.cgroups()
                    time.sleep(1.1)  # Complete the receiver's periodic counter snapshot.
                    status_after = f.raw_status()
                    received = status_after[target]['received_packets']-status_before[target]['received_packets']
                    accepted = status_after[source]['accepted_send_packets']
                    if accepted == 0: raise RuntimeError('No synthetic packets accepted; invalid load generator case')
                    window = cpu_window(cpu_before,cpu_after)
                    cpu_details(window,received*1280)
                    case = {'mode':mode,'pps_requested':pps,'reverse':reverse,'received':received,'accepted_send':accepted,
                            'receiver_fraction':received/accepted if accepted else None,'cpu':window,
                            'before':status_before,'after':status_after,'kernel_before':kernel_before,'kernel_after':f.kernel()}
                    result['cases'].append(case)
                    write_result(args.output,result)
                    print(f"{mode} {pps}pps {'reverse' if reverse else 'forward'}: recv={received} accepted={accepted} retention={case['receiver_fraction']}",flush=True)
                    f.start_agent(source)
                    wait_until('sender restored',lambda:source.lower() in f.raw_status()[target]['authenticated_peers'])
                    time.sleep(1.1)
            result['cleanup_errors'].extend(f.cleanup()); f=None
        result['status']='ok'
    except KeyboardInterrupt:
        result['status']='interrupted'
        raise
    except Exception as e:
        result['status'],result['error']='failed',str(e)
        raise
    finally:
        if f: result['cleanup_errors'].extend(f.cleanup())
        result['finished_at']=now()
        write_result(args.output,result)


if __name__ == '__main__':
    with performance_lock(): main()
