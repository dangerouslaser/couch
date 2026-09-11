"""Actual native child entrypoint checks, with no release payload or USB access."""
import json
import os
import queue
import socket
import subprocess
import threading
import unittest


@unittest.skipUnless(os.environ.get('COUCH_NATIVE_HOST_TEST'), 'native executable required')
class NativeEntrypoint(unittest.TestCase):
    def test_cancel_before_release_or_hardware_and_missing_config_is_visible(self):
        for cancel in (True, False):
            with self.subTest(cancel=cancel):
                executable=os.environ['COUCH_NATIVE_HOST_TEST']
                if os.name=='nt':
                    process=subprocess.Popen([executable,'--events-stdio'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                    incoming,outgoing=process.stdout,process.stdin
                    parent=child=None
                else:
                    parent,child=socket.socketpair()
                    # fd3 must survive subprocess's descriptor cleanup too.
                    process=subprocess.Popen([executable,'--events-fd','3'],pass_fds=tuple(set((3,child.fileno()))),preexec_fn=lambda:os.dup2(child.fileno(),3),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                    child.close();child=None
                    incoming=outgoing=parent.makefile('rwb',buffering=0)
                events=queue.Queue()
                def reader():
                    try:
                        while line:=incoming.readline():events.put(json.loads(line))
                    finally:events.put(None)
                thread=threading.Thread(target=reader,daemon=True);thread.start()
                received=[]
                try:
                    while True:
                        value=events.get(timeout=15)
                        if value is None:break
                        received.append(value)
                        if value['event']=='prompt':
                            option=next(v for v in value['options'] if v['label']=='Cancel') if cancel else value['options'][0]
                            outgoing.write(json.dumps({'id':value['id'],'value':option['value']}).encode()+b'\n');outgoing.flush()
                        if value['event']=='finished':break
                    process.wait(timeout=15)
                    self.assertEqual(process.returncode,0 if cancel else 1)
                    self.assertEqual(received[-1]['event'],'finished')
                    if not cancel:self.assertTrue(any('release configuration' in v.get('detail','') for v in received))
                finally:
                    if process.poll() is None:process.kill();process.wait(timeout=5)
                    incoming.close()
                    if outgoing is not incoming:outgoing.close()
                    if parent is not None:parent.close()
                    process.stderr.close()
                    if process.stdout is not incoming:process.stdout.close()

if __name__=='__main__':unittest.main()
