"""Receiver-window diagnostics. Never replace full-window goodput with a cropped peak."""
import math
import statistics


def receiver_windows(case):
    raw=case['raw_client'] if case['reverse'] else case['raw_server']
    full=case['raw_client']['end']['sum_received']
    seconds=float(full['seconds'])
    if not math.isfinite(seconds) or seconds<=0:raise ValueError('Invalid full receiver duration')
    zero=[];active=[];inconsistent=[]
    for index,interval in enumerate(raw['intervals']):
        row=interval['sum']
        if row.get('omitted'):continue
        if row.get('sender') is not False:raise ValueError('Expected receiver intervals')
        duration=float(row['seconds']);start=float(row['start']);end=float(row['end'])
        # This iperf build can emit an inconsistent first post-omit interval.
        # Preserve/report it rather than treating its misleading half-rate as
        # steady throughput or inventing elapsed time from conflicting fields.
        if not all(math.isfinite(x) for x in [duration,start,end]) or duration<=0 or abs(end-start-duration)>.002:
            inconsistent.append(index);continue
        if row['bytes']==0:zero.append({'start':start,'end':end,'seconds':duration})
        else:active.append(row['bytes']*8/duration/1e6)
    blackout=sum(row['seconds'] for row in zero)
    if blackout>seconds+.01:raise ValueError('Zero intervals exceed full receiver window')
    nonzero_seconds=max(0,seconds-blackout)
    return {
        'receiver_trace':'client' if case['reverse'] else 'server',
        'full_seconds':seconds,'full_receiver_mbps':full['bits_per_second']/1e6,
        'zero_byte_receiver_intervals':zero,'zero_byte_seconds':blackout,
        'zero_byte_window_fraction':blackout/seconds,
        'nonzero_window_equivalent_mbps':full['bytes']*8/nonzero_seconds/1e6 if nonzero_seconds else None,
        'nonzero_valid_interval_median_mbps':statistics.median(active) if active else None,
        'nonzero_valid_interval_min_mbps':min(active) if active else None,
        'nonzero_valid_interval_max_mbps':max(active) if active else None,
        'inconsistent_interval_indices':inconsistent,
        'scope':'1s-resolution receiver zero-byte intervals; nonzero equivalent/median are diagnostic steady-state references including ramps, not a replacement A/B result or exact packet outage duration',
    }
