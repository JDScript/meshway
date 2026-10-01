#!/usr/bin/env python3
"""Real managed Manager/TUN/lease generation using fresh Rust contract fixtures."""
import argparse
import json
import os
import math
import re
import statistics
import subprocess
from pathlib import Path
import signal
import tempfile
import time

from run import (TunnelFixture, Fixture, architecture, benchmark_case, bounded_integer, cpu_details, cpu_window,
                 docker, docker_json, interrupted, now, performance_lock, platform,
                 wait_until, write_result, PROCESS, environment, counter_delta)
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

RTT_SAMPLE = r"""
import json, os, subprocess, sys, time
from pathlib import Path
os.umask(0o077)
time.sleep(3)
command=['ping','-n','-I',sys.argv[1],'-i','0.5','-c','32','-w','19',sys.argv[2]]
start=time.monotonic_ns()
p=subprocess.run(command,capture_output=True,text=True,timeout=21,check=False)
r={'command':command,'started_monotonic_ns':start,'finished_monotonic_ns':time.monotonic_ns(),
   'exit_code':p.returncode,'stdout':p.stdout,'stderr':p.stderr,
   'scope':'NYC->SHA->NYC ICMP RTT while iperf runs; delayed 3s after client launch, not idle RTT'}
path=Path(sys.argv[3]); temporary=path.with_suffix('.tmp')
temporary.write_text(json.dumps(r)); temporary.replace(path)
"""


def summarize_loaded_rtt(raw):
    samples=[float(value) for value in re.findall(r'\btime=([0-9.]+) ms',raw['stdout'])]
    counts=re.search(r'(\d+) packets transmitted, (\d+) received',raw['stdout'])
    return {'samples_ms':samples,'median_ms':statistics.median(samples) if samples else None,
            'p95_ms':sorted(samples)[math.ceil(.95*len(samples))-1] if samples else None,
            'max_ms':max(samples) if samples else None,
            'upper_bound_samples':len(re.findall(r'\btime<',raw['stdout'])),
            'transmitted':int(counts[1]) if counts else None,'received':int(counts[2]) if counts else None}


class ManagedFixture(TunnelFixture):
    def exec(self,node,*command,**kwargs):
        if getattr(self,'rtt_request',None) and command[0]=='iperf3' and '--client' in command:
            source,target,path=self.rtt_request
            docker('exec','--detach',self.nodes['NYC'],'python3','-c',RTT_SAMPLE,source,target,path)
            self.rtt_request=None
        return super().exec(node,*command,**kwargs)

    def runtime_statistics(self):
        result={}
        for n in self.nodes:
            response=self.exec(n,'cat',RUN_DIRECTORY+'/runtime-statistics.json',check=False)
            if response.returncode==0: result[n]={'statistics':json.loads(response.stdout)}
        return result

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
        if self.args.tun_queue_length is not None:
            for n in order: self.exec(n,'ip','link','set','dev','mwlab0','txqueuelen',str(self.args.tun_queue_length))
            links={n:json.loads(self.exec(n,'ip','-json','link','show','dev','mwlab0').stdout) for n in order}
            if any(link['txqlen']!=self.args.tun_queue_length for rows in links.values() for link in rows):
                raise RuntimeError('Requested isolated TUN queue length did not take effect')
        if self.args.require_runtime_statistics:
            wait_until('ignored fixture Runtime statistics sidecars',lambda:len(self.runtime_statistics())==len(order),timeout=10)
        if mode=='relay' and self.probe('NYC',self.address('SHA','b')): raise RuntimeError('Unexpected relay bypass')
        return ips,{'snapshot':self.snapshot(),'connections':self.connections(),
                    'binary_sha256':self.exec('NYC','sha256sum','/usr/local/bin/meshway-managed-measure').stdout.split()[0],
                    'offload':{n:{dev:self.exec(n,'ethtool','-k',dev,check=False).stdout for dev in ['eth0','mwlab0']} for n in order}}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--image',required=True); p.add_argument('--output',type=Path,required=True)
    p.add_argument('--temp-dir',type=Path,required=True); p.add_argument('--seconds',type=bounded_integer(1,30),default=10)
    p.add_argument('--iterations',type=bounded_integer(1,3),default=1); p.add_argument('--modes',nargs='+',choices=['direct','relay'],default=['direct','relay'])
    p.add_argument('--tun-queue-length',type=int,choices=[32,128,256],default=None,help='Same-image namespace-only TUN queue A/B; omit to retain binary default')
    p.add_argument('--require-runtime-statistics',action='store_true',help='Fail unless this measurement image publishes experiment-only Runtime statistics')
    p.add_argument('--parallel',type=int,nargs='+',choices=[1,4,8],default=[1,4],help='Select only required stream counts')
    p.add_argument('--directions',nargs='+',choices=['forward','reverse'],default=['forward','reverse'])
    p.add_argument('--loaded-rtt',action='store_true',help='32 delayed ICMP probes during each iperf workload')
    args=p.parse_args(); args.cpus,args.memory,args.mtu,args.omit=1.,'256m',1500,2
    if len(args.parallel)*len(args.directions)*(args.seconds+args.omit)*args.iterations>210: raise ValueError('Case duration exceeds bounded managed fixture lease budget')
    if args.loaded_rtt and args.seconds<20: raise ValueError('Loaded RTT sampling requires at least 20 measured seconds')
    os.umask(0o077)
    if args.output.exists(): raise ValueError('Output exists')
    signal.signal(signal.SIGTERM,interrupted)
    info=docker_json('info','--format','{{json .}}'); native=architecture(info['Architecture'])
    if native!=architecture(platform.machine()): raise ValueError('Native Docker required')
    result={'run_id':'meshway-managed-throughput-'+os.urandom(8).hex(),'status':'running','started_at':now(),
        'settings':{k:str(v) if isinstance(v,Path) else v for k,v in vars(args).items()},
        'scope':'Real managed Manager/coherent contract compilation/TUN generation/300-second lease watchdog; fixture keys, no controller enrollment or HTTP refresh. E2E plus NativeUDP hop Noise, full 64-byte node IDs, MTU1000, two Tokio workers.',
        'environment':environment(info),
        'source_revision':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
        'source_dirty':bool(subprocess.check_output(['git','status','--porcelain'],text=True).strip()),
        'packet_pump':'meshway_dataplane::packet_io::run_with_activation (shared pump)',
        'cases':[],'fixtures':{},'cleanup_errors':[]}
    f=None
    try:
        for mode in args.modes:
            f=ManagedFixture(args,result); f.prepare_image(native)
            ips,result['fixtures'][mode]=f.setup_managed(mode)
            for iteration in range(1,args.iterations+1):
                for parallel in args.parallel:
                    for direction in args.directions:
                        reverse=direction=='reverse'
                        cpu_before=f.cgroups(); kernel_before=f.kernel(); runtime_before=f.runtime_statistics()
                        processes={n:json.loads(f.exec(n,'python3','-c',PROCESS).stdout) for n in f.nodes}
                        rtt_file=f"{RUN_DIRECTORY}/rtt-p{parallel}-{direction}-{iteration}.json"
                        if args.loaded_rtt: f.rtt_request=(ips['NYC'],ips['SHA'],rtt_file)
                        case=benchmark_case(f,ips['NYC'],ips['SHA'],'tcp',parallel,reverse,iteration)
                        if args.loaded_rtt:
                            wait_until('loaded RTT sample completion',lambda:f.exec('NYC','test','-f',rtt_file,check=False).returncode==0,timeout=5)
                            case['loaded_rtt']=json.loads(f.exec('NYC','cat',rtt_file).stdout)
                            case['loaded_rtt']['summary']=summarize_loaded_rtt(case['loaded_rtt'])
                        runtime_after=f.runtime_statistics()
                        case.update(mode=mode,mtu=1000,hop_count=2 if mode=='relay' else 1,
                            kernel_before=kernel_before,kernel_after=f.kernel(),process_before=processes,
                            process_after={n:json.loads(f.exec(n,'python3','-c',PROCESS).stdout) for n in f.nodes},
                            agent_before=runtime_before,agent_after=runtime_after,
                            agent_counter_delta=counter_delta(runtime_before,runtime_after))
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
