#!/usr/bin/env python3
"""Condense retained raw artifacts into throughput, CPU and exclusive loss evidence."""
import argparse
import json
from pathlib import Path
from run import cpu_details


def snmp(text):
    lines=text.splitlines()
    result={}
    for names,values in zip(lines[::2],lines[1::2]):
        a,b=names.split(),values.split()
        if a and b and a[0]==b[0]:
            result[a[0].rstrip(':')]=dict(zip(a[1:],map(int,b[1:])))
    return result


def kernel_delta(case,node):
    before=snmp(case.get('kernel_before',{}).get(node,{}).get('proc',{}).get('files',{}).get('snmp',
                case.get('process_before',{}).get(node,{}).get('files',{}).get('net/snmp','')))
    after=snmp(case.get('kernel_after',{}).get(node,{}).get('proc',{}).get('files',{}).get('snmp',
               case.get('process_after',{}).get(node,{}).get('files',{}).get('net/snmp','')))
    return {f'{section}.{key}':after[section][key]-before[section][key] if key in before.get(section,{}) and key in after.get(section,{}) else None
            for section,keys in {'Udp':['RcvbufErrors','SndbufErrors','InErrors'],'Ip':['FragCreates','FragFails','ReasmFails'],
                                 'Tcp':['RetransSegs']}.items() for key in keys}


def describe(case):
    window=case.get('cpu',{})
    received_bytes=case.get('raw_client',{}).get('end',{}).get('sum_received',{}).get('bytes')
    if 'total_cpu_ns_per_receiver_payload_byte' not in window and 'before' in window and 'after' in window and received_bytes is not None:
        cpu_details(window,received_bytes)
    row={k:case.get(k) for k in ['mode','mtu','id','pps_requested','received','accepted_send','receiver_fraction','reverse'] if k in case}
    row['mbps']=case.get('summary',{}).get('receiver_bits_per_second',0)/1e6
    start=case.get('raw_client',{}).get('start',{})
    row['tcp_mss_bytes']=start.get('tcp_mss_default')
    row['iperf_block_bytes']=start.get('test_start',{}).get('blksize')
    row['socket_buffers']={k:start.get(k) for k in ['sock_bufsize','sndbuf_actual','rcvbuf_actual']}
    row['retransmits']=case.get('summary',{}).get('retransmits')
    row['cpu_ns_per_byte_upper_estimate']=case.get('cpu',{}).get('total_cpu_ns_per_receiver_payload_byte')
    row['nodes']={}
    for node,cpu in case.get('cpu',{}).get('nodes',{}).items():
        entry={'cpu_core_percent':cpu.get('cpu_percent_one_core'),'user_usec':cpu.get('user_usec_delta'),
               'system_usec':cpu.get('system_usec_delta'),'throttled_usec':cpu.get('throttled_usec_delta'),
               'kernel_delta':kernel_delta(case,node)}
        delta=case.get('agent_counter_delta',{}).get(node,{}).get('statistics',{})
        entry['runtime_drops']=delta.get('native_udp',{}).get('drop_reasons',{})
        entry['runtime_counters']=delta.get('native_udp',{})
        link_samples=[]
        for side in ['kernel_before','kernel_after']:
            link_samples.append({link['ifname']:link for link in case.get(side,{}).get(node,{}).get('links',[])})
        entry['links']={}
        for name,link in link_samples[1].items():
            old=link_samples[0].get(name,{})
            first=old.get('stats64',old.get('stats',{})); second=link.get('stats64',link.get('stats',{}))
            entry['links'][name]={'tx_queue_length':link.get('txqlen'),'mtu':link.get('mtu'),
                **{f'{direction}_{field}':second[direction][field]-first[direction][field]
                    for direction in ['rx','tx'] for field in ['packets','bytes','dropped','errors']
                    if field in first.get(direction,{}) and field in second.get(direction,{})}}
        entry['tun_counters']=case.get('agent_counter_delta',{}).get(node,{}).get('tun',{})
        samples=[]
        for side in ['process_before','process_after']:
            files=case.get(side,{}).get(node,{}).get('files',{})
            samples.append(dict(line.split(':',1) for line in files.get('io','').splitlines() if ':' in line))
            if side=='process_after':
                status=dict(line.split(':',1) for line in files.get('status','').splitlines() if ':' in line)
                entry['rss']=status.get('VmRSS','').strip()
        entry['read_write_syscall_delta']={k:int(samples[1][k])-int(samples[0][k]) for k in ['syscr','syscw'] if k in samples[0] and k in samples[1]}
        row['nodes'][node]=entry
    return row


def main():
    p=argparse.ArgumentParser(description=__doc__); p.add_argument('artifacts',type=Path,nargs='+')
    args=p.parse_args()
    for path in args.artifacts:
        raw=json.loads(path.read_text())
        print(json.dumps({'artifact':str(path),'status':raw['status'],'image':raw.get('image'),
                          'cleanup_errors':raw.get('cleanup_errors'),
                          'cases':[describe(c) for c in raw['cases']]},indent=2))


if __name__=='__main__': main()
