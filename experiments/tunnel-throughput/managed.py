#!/usr/bin/env python3
"""Real managed Manager/TUN/lease generation using fresh Rust contract fixtures."""
import argparse
import json
import os
from pathlib import Path
import signal
import tempfile
import time

from run import (TunnelFixture, Fixture, architecture, benchmark_case, bounded_integer, cpu_details, cpu_window,
                 docker, docker_json, interrupted, now, performance_lock, platform,
                 wait_until, write_result, PROCESS, environment)
from meshway import RUN_DIRECTORY, HERE

SUPERVISE = r"""
import subprocess, sys, os
sys.path.insert(0, '/run/meshway-lab')
import meshway_node as m
os.environ['MESHWAY_THROUGHPUT_CONFIG']='/run/meshway-lab/config.json'
os.environ['MESHWAY_THROUGHPUT_STATUS']='/run/meshway-lab/status.json'
c=subprocess.Popen(['meshway-managed-measure','--exact','managed_dataplane::throughput::serve','--ignored','--nocapture'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
s=m.process_identity(c.pid)
if s: m.save('agent-pid.json',{'pid':c.pid,'start_ticks':s['start_ticks']})
m.save('agent-exit.json',{'exit_code':c.wait()})
"""


class ManagedFixture(TunnelFixture):
    def create_node(self,node,network,topology,forward=False):
        name=f'{self.prefix}-{topology}-{node.lower()}'
        self.containers_owned.append(name); self.nodes[node]=name
        docker('run','--detach','--name',name,*self.labels(),'--network',self.networks[network]['name'],
               '--cap-drop','ALL','--cap-add','NET_ADMIN','--cap-add','NET_RAW','--device','/dev/net/tun',
               '--security-opt','no-new-privileges','--read-only','--tmpfs','/run:rw,nosuid,nodev,noexec,mode=0700,size=32m',
               '--tmpfs','/tmp:rw,nosuid,nodev,noexec,mode=0700,size=16m','--cpus',str(self.args.cpus),
               '--memory',self.args.memory,'--memory-swap',self.args.memory,'--pids-limit','64',
               '--sysctl','net.ipv4.ip_forward=0','--sysctl','net.ipv6.conf.all.disable_ipv6=1',
               '--sysctl','net.ipv6.conf.default.disable_ipv6=1','--entrypoint','python3',self.image,
               '-c','import signal; signal.pause()')
        self.exec(node,'mkdir','-m','700',RUN_DIRECTORY)
        for f in ['node.py','meshway_node.py']: self.copy_file(node,HERE/f,f'{RUN_DIRECTORY}/{f}')
        docker('exec','--detach',name,'python3',f'{RUN_DIRECTORY}/node.py','serve')
        wait_until('node helper',lambda:self.probe(node,'127.0.0.1'))

    def start_agent(self,node):
        self.agents_started.add(node)
        docker('exec','--detach',self.nodes[node],'python3','-c',SUPERVISE)

    def setup_managed(self,mode):
        self.create_network('a','managed-'+mode)
        self.create_node('NYC','a','managed-'+mode)
        if mode=='relay':
            self.create_network('b','managed-relay')
            self.create_node('LAX','a','managed-relay')
            docker('network','connect',self.networks['b']['name'],self.nodes['LAX'])
        self.create_node('SHA','b' if mode=='relay' else 'a','managed-'+mode)
        self.remove_defaults()
        order=['NYC','LAX','SHA'] if mode=='relay' else ['NYC','SHA']
        endpoints=','.join(self.address(n,'b' if n in ['LAX','SHA'] and mode=='relay' else 'a')+':41000' for n in order)
        with tempfile.TemporaryDirectory(dir=self.args.temp_dir,prefix='managed-throughput-keys-') as d:
            directory=Path(d)
            generator=self.prefix+'-generator'; self.containers_owned.append(generator)
            response=docker('run','--name',generator,*self.labels(),'--network','none','--cap-drop','ALL',
                '--read-only','--security-opt','no-new-privileges','--user',f'{os.getuid()}:{os.getgid()}',
                '--cpus','1','--memory','256m','--memory-swap','256m','--pids-limit','64',
                '--env','MESHWAY_THROUGHPUT_DIRECTORY=/lab','--env','MESHWAY_THROUGHPUT_ENDPOINTS='+endpoints,
                '--mount',f'type=bind,source={directory},target=/lab',self.image,
                '--exact','managed_dataplane::throughput::generate','--ignored',check=False)
            if response.returncode: raise RuntimeError('Private managed fixture generation failed; output suppressed')
            docker('rm',generator)
            for i,n in enumerate(order): self.copy_file(n,directory/f'node-{i}.json',RUN_DIRECTORY+'/config.json')
        for n in order: self.start_agent(n)
        ips={'NYC':'198.18.1.1','SHA':f'198.18.1.{len(order)}'}
        for n in order:
            def ready(node=n):
                state=self.agent_state(node)
                if not state['pid_recorded']: return False
                if not state['running']: raise RuntimeError('Managed generation exited; private diagnostics suppressed')
                response=self.exec(node,'ip','-json','address','show','dev','mwlab0',check=False)
                return response.returncode==0 and any('UP' in link['flags'] for link in json.loads(response.stdout))
            wait_until('real managed TUN generation',ready,timeout=40)
        for n,peer in [('NYC','SHA'),('SHA','NYC')]:
            wait_until('managed TUN ICMP',lambda n=n,peer=peer:self.exec(n,'ping','-n','-c','1','-W','1',ips[peer],check=False).returncode==0,timeout=40)
        if mode=='relay' and self.probe('NYC',self.address('SHA','b')): raise RuntimeError('Unexpected relay bypass')
        return ips,{'snapshot':self.snapshot(),'connections':self.connections(),
                    'offload':{n:{dev:self.exec(n,'ethtool','-k',dev,check=False).stdout for dev in ['eth0','mwlab0']} for n in order}}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image',required=True); p.add_argument('--output',type=Path,required=True)
    p.add_argument('--temp-dir',type=Path,required=True); p.add_argument('--seconds',type=bounded_integer(1,30),default=10)
    p.add_argument('--iterations',type=bounded_integer(1,3),default=1); p.add_argument('--modes',nargs='+',choices=['direct','relay'],default=['direct','relay'])
    args=p.parse_args(); args.cpus,args.memory,args.mtu,args.omit=1.,'256m',1500,2
    if 4*(args.seconds+args.omit)*args.iterations>210: raise ValueError('Case duration exceeds bounded managed fixture lease budget')
    os.umask(0o077)
    if args.output.exists(): raise ValueError('Output exists')
    signal.signal(signal.SIGTERM,interrupted)
    info=docker_json('info','--format','{{json .}}'); native=architecture(info['Architecture'])
    if native!=architecture(platform.machine()): raise ValueError('Native Docker required')
    result={'run_id':'meshway-managed-throughput-'+os.urandom(8).hex(),'status':'running','started_at':now(),
        'settings':{k:str(v) if isinstance(v,Path) else v for k,v in vars(args).items()},
        'scope':'Real managed Manager/coherent contract compilation/TUN generation/300-second lease watchdog; fixture keys, no controller enrollment or HTTP refresh. E2E plus NativeUDP hop Noise, full 64-byte node IDs, MTU1000, two Tokio workers.',
        'environment':environment(info),
        'cases':[],'fixtures':{},'cleanup_errors':[]}
    f=None
    try:
        for mode in args.modes:
            f=ManagedFixture(args,result); f.prepare_image(native)
            ips,result['fixtures'][mode]=f.setup_managed(mode)
            for iteration in range(1,args.iterations+1):
                for parallel in [1,4]:
                    for reverse in [False,True]:
                        cpu_before=f.cgroups(); kernel_before=f.kernel()
                        processes={n:json.loads(f.exec(n,'python3','-c',PROCESS).stdout) for n in f.nodes}
                        case=benchmark_case(f,ips['NYC'],ips['SHA'],'tcp',parallel,reverse,iteration)
                        case.update(mode=mode,mtu=1000,hop_count=2 if mode=='relay' else 1,
                            kernel_before=kernel_before,kernel_after=f.kernel(),process_before=processes,
                            process_after={n:json.loads(f.exec(n,'python3','-c',PROCESS).stdout) for n in f.nodes})
                        case['cpu']=cpu_window(cpu_before,f.cgroups())
                        cpu_details(case['cpu'],case['raw_client']['end']['sum_received']['bytes'])
                        result['cases'].append(case); write_result(args.output,result)
                        print(f"managed {mode} {case['id']}: {case['summary']['receiver_bits_per_second']/1e6:.2f} Mbit/s retrans={case['summary']['retransmits']}",flush=True)
                        if case['status']!='ok': raise RuntimeError('Managed iperf failed')
            if mode=='relay':
                # Stop only relay generation; endpoint helpers remain up.
                f.agent_helper('LAX','stop')
                wait_until('relay stopped',lambda:not f.agent_state('LAX')['running'])
                result['relay_stop_blocks_both_directions']=all(f.exec(n,'ping','-n','-c','1','-W','1',ips[peer],check=False).returncode!=0 for n,peer in [('NYC','SHA'),('SHA','NYC')])
                if not result['relay_stop_blocks_both_directions']: raise RuntimeError('Relay bypass detected')
            result['cleanup_errors'].extend(f.cleanup()); f=None
        result['status']='ok'
    except KeyboardInterrupt:
        result['status']='interrupted'
        raise
    except Exception as e:
        result['status'],result['error']='failed',str(e); raise
    finally:
        if f: result['cleanup_errors'].extend(f.cleanup())
        result['finished_at']=now(); write_result(args.output,result)


if __name__=='__main__':
    with performance_lock(): main()
